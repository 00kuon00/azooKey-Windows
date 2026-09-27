mod ipc;
mod suggestion;

use serde::{Deserialize, Serialize};
use shared::{AppConfig, UserDictionaryEntry};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{path::PathBuf, sync::Mutex};
use tauri::Emitter;

#[derive(Debug)]
pub struct AppState {
    settings: Mutex<AppConfig>,
    ipc: ipc::IPCService,
    // ユーザー辞書の登録候補（#17）を探している最中か、止めるよう頼まれたか
    suggestion_running: Arc<AtomicBool>,
    suggestion_cancel: Arc<AtomicBool>,
}

impl AppState {
    fn new() -> Self {
        AppState {
            settings: Mutex::new(AppConfig::new()),
            ipc: ipc::IPCService::new().unwrap(),
            suggestion_running: Arc::new(AtomicBool::new(false)),
            suggestion_cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> AppConfig {
    let config = state.settings.lock().unwrap();
    config.clone()
}

#[tauri::command]
fn update_config(state: tauri::State<AppState>, new_config: AppConfig) {
    let mut config = state.settings.lock().unwrap();
    *config = new_config;
    config.write();

    state.ipc.clone().update_config().unwrap();
}

#[derive(Debug, Serialize)]
struct UserDictionary {
    entries: Vec<UserDictionaryEntry>,
    // 読みに辞書で使えない文字があって、変換に使えなかった語
    unregistered: Vec<UserDictionaryEntry>,
}

#[tauri::command]
fn get_user_dictionary(state: tauri::State<AppState>) -> Result<UserDictionary, String> {
    let entries = UserDictionaryEntry::read_all().map_err(|e| e.to_string())?;
    // 元データが前回と同じならサーバは作り直さず、前回の結果を返す
    let unregistered = state
        .ipc
        .clone()
        .update_config()
        .map_err(|e| e.to_string())?;
    Ok(UserDictionary {
        entries,
        unregistered,
    })
}

#[tauri::command]
fn save_user_dictionary(
    state: tauri::State<AppState>,
    entries: Vec<UserDictionaryEntry>,
) -> Result<Vec<UserDictionaryEntry>, String> {
    UserDictionaryEntry::write_all(&entries).map_err(|e| e.to_string())?;
    state.ipc.clone().update_config().map_err(|e| e.to_string())
}

/// 変換エンジンへの問い合わせ（IPC）
struct IpcEngine(ipc::IPCService);

impl suggestion::Engine for IpcEngine {
    fn check(
        &mut self,
        queries: &[(String, String)],
    ) -> Result<Vec<suggestion::Convertibility>, String> {
        self.0
            .check_convertibility(queries)
            .map_err(|e| format!("変換エンジンに問い合わせられませんでした: {e}"))
    }
}

/// 探し終えたときに画面へ送る。失敗したときは error に理由が入る
#[derive(Debug, Clone, Serialize)]
struct SuggestionFinished {
    result: Option<suggestion::ScanResult>,
    error: Option<String>,
}

fn scan_suggestions(
    app: &tauri::AppHandle,
    ipc: ipc::IPCService,
    cancel: &AtomicBool,
) -> Result<suggestion::ScanResult, String> {
    let started = std::time::Instant::now();
    let mut progress = |p: suggestion::Progress| {
        let _ = app.emit("suggestion-progress", p);
    };
    // 材料を読む前に LLM に届くかを確かめる（読み終えてから「届かない」で止めない）
    progress(suggestion::Progress {
        phase: suggestion::Phase::Connect,
        done: 0,
        total: 0,
        found: 0,
    });
    let mut llm = suggestion::LlmClient::connect()?;

    let registered: HashSet<String> = UserDictionaryEntry::read_all()
        .map_err(|e| format!("ユーザー辞書を読めませんでした: {e}"))?
        .into_iter()
        .map(|entry| entry.word)
        .collect();
    let rejected: HashSet<String> = shared::read_rejected_suggestions()
        .map_err(|e| format!("却下した語を読めませんでした: {e}"))?
        .into_iter()
        .collect();

    let files = suggestion::material_files(
        &suggestion::default_claude_projects(),
        &suggestion::default_vault(),
    );
    let (counter, stopped) = suggestion::read_materials(&files, cancel, &mut progress);
    let (suggestions, mut stats, stopped) = if stopped {
        (
            Vec::new(),
            suggestion::ScanStats {
                texts: counter.texts,
                ..Default::default()
            },
            true,
        )
    } else {
        let mut engine = IpcEngine(ipc);
        suggestion::find_suggestions(
            &counter,
            &registered,
            &rejected,
            &mut engine,
            &mut llm,
            cancel,
            &mut progress,
        )?
    };
    stats.seconds = started.elapsed().as_secs_f64();
    stats.model = llm.model.clone();
    Ok(suggestion::ScanResult {
        stopped,
        suggestions,
        stats,
    })
}

/// 会話ログと Vault から登録候補を探し始める。画面を固めないよう別のスレッドで走らせ、
/// 進み具合は suggestion-progress、結果は suggestion-finished で送る。
/// （IPCService は自前の tokio ランタイムで block_on するので、tauri の非同期ランタイムの中では呼べない）
#[tauri::command]
fn start_suggestion_scan(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    if state.suggestion_running.swap(true, Ordering::SeqCst) {
        return Err("すでに探しています".to_string());
    }
    state.suggestion_cancel.store(false, Ordering::SeqCst);
    let running = state.suggestion_running.clone();
    let cancel = state.suggestion_cancel.clone();
    let ipc = state.ipc.clone();
    std::thread::spawn(move || {
        let finished = match scan_suggestions(&app, ipc, &cancel) {
            Ok(result) => SuggestionFinished {
                result: Some(result),
                error: None,
            },
            Err(error) => SuggestionFinished {
                result: None,
                error: Some(error),
            },
        };
        running.store(false, Ordering::SeqCst);
        let _ = app.emit("suggestion-finished", finished);
    });
    Ok(())
}

/// 探すのを止める。ここまでに見つかった候補は suggestion-finished で届く
#[tauri::command]
fn stop_suggestion_scan(state: tauri::State<AppState>) {
    state.suggestion_cancel.store(true, Ordering::SeqCst);
}

/// 却下した語を覚える（次から候補に出さない）
#[tauri::command]
fn reject_suggestions(words: Vec<String>) -> Result<(), String> {
    shared::add_rejected_suggestions(&words).map_err(|e| e.to_string())
}

#[tauri::command]
fn reset_learning(state: tauri::State<AppState>) -> Result<(), String> {
    state
        .ipc
        .clone()
        .reset_learning()
        .map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Capability {
    cpu: bool,
    cuda: bool,
    vulkan: bool,
}

#[tauri::command]
fn check_capability() -> Capability {
    // cuda:
    // cudart64_12.dll
    // cublas64_12.dll

    // vulkan:
    // vulkan-1.dllの存在確認

    let mut capability = Capability {
        cpu: true,
        cuda: false,
        vulkan: false,
    };

    // Check for CUDA availability
    let cuda_files = ["cudart64_12.dll", "cublas64_12.dll"];
    let cuda_available = cuda_files.iter().all(|file| {
        // Check if the file exists in system path or in the current directory
        std::env::var("PATH")
            .unwrap_or_default()
            .split(';')
            .map(PathBuf::from)
            .chain(std::iter::once(std::env::current_dir().unwrap_or_default()))
            .any(|path| path.join(file).exists())
    });
    capability.cuda = cuda_available;

    // Check for Vulkan availability
    let vulkan_file = "vulkan-1.dll";
    let vulkan_available = std::env::var("PATH")
        .unwrap_or_default()
        .split(';')
        .map(PathBuf::from)
        .chain(std::iter::once(std::env::current_dir().unwrap_or_default()))
        .any(|path| path.join(vulkan_file).exists());
    capability.vulkan = vulkan_available;

    capability
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState::new();

    tauri::Builder::default()
        .manage(app_state)
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            get_config,
            update_config,
            reset_learning,
            get_user_dictionary,
            save_user_dictionary,
            start_suggestion_scan,
            stop_suggestion_scan,
            reject_suggestions,
            check_capability
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
