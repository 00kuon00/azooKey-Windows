mod ipc;

use serde::{Deserialize, Serialize};
use shared::{AppConfig, UserDictionaryEntry};
use std::{path::PathBuf, sync::Mutex};

#[derive(Debug)]
pub struct AppState {
    settings: Mutex<AppConfig>,
    ipc: ipc::IPCService,
}

impl AppState {
    fn new() -> Self {
        AppState {
            settings: Mutex::new(AppConfig::new()),
            ipc: ipc::IPCService::new().unwrap(),
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
            check_capability
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
