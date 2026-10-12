//! 日英の自動判定（#26）の設定: 自分の語彙を作り直す・英字の候補に入れる語と入れない語の一覧。
//! 会話ログ・Vault は読むだけで書き換えない。重い判定は常駐の変換エンジンを止めないよう、
//! 別のプロセス（azookey-server.exe --build-english-vocabulary）で行う

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use crate::suggestion::{user_texts_from_log_line, Source};

/// 自分の語彙に入れる回数の下限（#17 の登録の提案と同じ）
pub const MIN_COUNT: usize = 3;

/// 子のプロセスにコンソールの窓を作らせない（Win32 の CREATE_NO_WINDOW）
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static CODE_BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```.*?```").unwrap());
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]*`").unwrap());
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").unwrap());
static LATIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z]+").unwrap());
static JAPANESE_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[ぁ-ゖァ-ヺー\p{Han}々〆ヶ]+").unwrap());

/// 英字の語と続けて書かれると、1 つの技術的な語（パス・ファイル名・識別子）や英文の語の一部（you'll の ll）とみなす文字
fn joins(c: char) -> bool {
    c.is_ascii_digit() || "_./\\-@#$%+:'’".contains(c)
}

/// 人が続けて打った部分だけを残す（コード・URL は打ち方が違うので外す）
fn clean(text: &str) -> String {
    let text = CODE_BLOCK.replace_all(text, "\n");
    let text = INLINE_CODE.replace_all(&text, " ");
    URL.replace_all(&text, " ").into_owned()
}

/// 英字の語（2 文字以上・小文字にまとめる）を数える。前後の文字とつながった語は数えない
fn count_words(text: &str, counts: &mut HashMap<String, usize>) {
    for found in LATIN.find_iter(text) {
        let before = text[..found.start()].chars().next_back();
        let after = text[found.end()..].chars().next();
        if found.as_str().len() < 2 || before.is_some_and(joins) || after.is_some_and(joins) {
            continue;
        }
        *counts.entry(found.as_str().to_lowercase()).or_default() += 1;
    }
}

/// 変換エンジンに渡す材料（server-swift の EnglishVocabularyInput と同じ形）
#[derive(Debug, Default, Serialize)]
pub struct Material {
    pub counts: HashMap<String, usize>,
    pub japanese: Vec<String>,
}

/// 会話ログの 1 行が人の打った発言（promptSource が typed / queued）か。
/// ⚠ ログは JSON を詰めて書いてあるので、文字列で見分ける（4GB 近くあり、すべてを JSON として読むと遅い）
fn is_typed(line: &str) -> bool {
    line.contains("\"promptSource\":\"typed\"") || line.contains("\"promptSource\":\"queued\"")
}

/// 材料を読む。英字の語は会話ログのユーザー発言（#17 と同じ範囲）と Vault のノートから数え、
/// 日本語の部分は人が打った発言からだけ集める。止められたら None
pub fn read_material(
    files: &[(PathBuf, Source)],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Option<Material> {
    let mut material = Material::default();
    let mut seen = HashSet::new();
    let total: u64 = files
        .iter()
        .filter_map(|(path, _)| path.metadata().ok())
        .map(|m| m.len())
        .sum();
    let mut done = 0u64;
    let mut last_report = Instant::now();
    for (path, source) in files {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(file) = std::fs::File::open(path) else {
            continue;
        };
        match source {
            Source::Conversation => {
                let mut reader = BufReader::with_capacity(1 << 20, file);
                let mut line = Vec::new();
                loop {
                    line.clear();
                    match reader.read_until(b'\n', &mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => done += n as u64,
                    }
                    let line = String::from_utf8_lossy(&line);
                    let typed = is_typed(&line);
                    for text in user_texts_from_log_line(&line) {
                        let text = clean(&text);
                        count_words(&text, &mut material.counts);
                        if typed {
                            for run in JAPANESE_RUN.find_iter(&text) {
                                if seen.insert(run.as_str().to_string()) {
                                    material.japanese.push(run.as_str().to_string());
                                }
                            }
                        }
                    }
                    if last_report.elapsed() > Duration::from_millis(100) {
                        if cancel.load(Ordering::Relaxed) {
                            return None;
                        }
                        progress(done, total);
                        last_report = Instant::now();
                    }
                }
            }
            Source::Note => {
                let mut bytes = Vec::new();
                if BufReader::new(file).read_to_end(&mut bytes).is_ok() {
                    done += bytes.len() as u64;
                    count_words(
                        &clean(&String::from_utf8_lossy(&bytes)),
                        &mut material.counts,
                    );
                }
            }
        }
    }
    progress(total, total);
    material.counts.retain(|_, count| *count >= MIN_COUNT);
    Some(material)
}

/// 自分の語彙のファイル（%APPDATA%\Azookey\english_vocabulary.json。server-swift の EnglishVocabularyFile と同じ形）
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyFile {
    pub words: BTreeMap<String, usize>,
    #[serde(default)]
    pub removed: BTreeMap<String, usize>,
    #[serde(default)]
    pub built_at: String,
}

/// F10・F6・帯・設定画面で覚えた向き（%APPDATA%\Azookey\english_learned.json）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    English,
    Japanese,
}

pub fn config_root() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("Azookey")
}

pub fn vocabulary_path(root: &Path) -> PathBuf {
    root.join("english_vocabulary.json")
}

pub fn learned_path(root: &Path) -> PathBuf {
    root.join("english_learned.json")
}

pub fn read_vocabulary(root: &Path) -> Option<VocabularyFile> {
    let text = std::fs::read_to_string(vocabulary_path(root)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn read_learned(root: &Path) -> BTreeMap<String, Direction> {
    std::fs::read_to_string(learned_path(root))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// 一時ファイルに書いてから置き換える（途中で落ちても元のファイルを壊さない）
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

/// 語の向きを変える。`direction` が None ならその語の覚えた向きを消す（「戻す」）
pub fn set_learned(root: &Path, word: &str, direction: Option<Direction>) -> std::io::Result<()> {
    let mut learned = read_learned(root);
    let key = word.to_lowercase();
    match direction {
        Some(direction) => {
            learned.insert(key, direction);
        }
        None => {
            learned.remove(&key);
        }
    }
    std::fs::create_dir_all(root)?;
    write_atomic(
        &learned_path(root),
        &serde_json::to_string_pretty(&learned).map_err(std::io::Error::other)?,
    )
}

/// 設定画面に出す語
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WordRow {
    pub word: String,
    /// "own" = 自分の語彙 / "learned" = 覚えた語（F10・帯で選んだ）/ "excluded" = 英字にしない語（F6・外した）
    pub source: &'static str,
    /// 自分の語彙での回数（無ければ 0）
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct WordList {
    pub vocabulary: Option<VocabularySummary>,
    /// 英字の候補に入れる語（回数の多い順。覚えた語が先）
    pub english: Vec<WordRow>,
    /// 英字の候補に入れない語
    pub excluded: Vec<WordRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularySummary {
    pub words: usize,
    pub removed: usize,
    pub built_at: String,
}

pub fn word_list(root: &Path) -> WordList {
    let vocabulary = read_vocabulary(root);
    let learned = read_learned(root);
    let counts = vocabulary.as_ref().map(|v| &v.words);
    let count_of = |word: &str| counts.and_then(|c| c.get(word)).copied().unwrap_or(0);
    let mut english: Vec<WordRow> = learned
        .iter()
        .filter(|(_, direction)| **direction == Direction::English)
        .map(|(word, _)| WordRow {
            word: word.clone(),
            source: "learned",
            count: count_of(word),
        })
        .collect();
    if let Some(counts) = counts {
        let mut own: Vec<WordRow> = counts
            .iter()
            .filter(|(word, _)| !learned.contains_key(word.as_str()))
            .map(|(word, count)| WordRow {
                word: word.clone(),
                source: "own",
                count: *count,
            })
            .collect();
        own.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.word.cmp(&b.word)));
        english.extend(own);
    }
    let excluded = learned
        .iter()
        .filter(|(_, direction)| **direction == Direction::Japanese)
        .map(|(word, _)| WordRow {
            word: word.clone(),
            source: "excluded",
            count: count_of(word),
        })
        .collect();
    WordList {
        vocabulary: vocabulary.map(|v| VocabularySummary {
            words: v.words.len(),
            removed: v.removed.len(),
            built_at: v.built_at,
        }),
        english,
        excluded,
    }
}

/// 進み具合（画面の english-vocabulary-progress）
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    /// "read" = 会話ログとノートを読んでいる / "readings" = 日本語の部分に読みを付けている / "words" = 語を確かめている
    pub phase: String,
    pub done: u64,
    pub total: u64,
}

/// 変換エンジン（azookey-server.exe）の場所。開発中は環境変数 AZOOKEY_SERVER_EXE で変えられる
pub fn server_exe(root: &Path) -> PathBuf {
    std::env::var("AZOOKEY_SERVER_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("azookey-server.exe"))
}

/// 別のプロセスで自分の語彙を作り、%APPDATA%\Azookey\english_vocabulary.json に書く。
/// 進み具合の行（progress <段階> <済んだ数> <全体の数>）を `progress` に渡す。止められたら Ok(None)
pub fn build_vocabulary(
    material: &Material,
    root: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Option<VocabularyFile>, String> {
    let exe = server_exe(root);
    let exe_dir = exe
        .parent()
        .ok_or("変換エンジンの場所が分かりません")?
        .to_path_buf();
    let work = std::env::temp_dir().join(format!("azookey-english-{}", std::process::id()));
    std::fs::create_dir_all(&work).map_err(|e| format!("一時フォルダを作れませんでした: {e}"))?;
    let input = work.join("input.json");
    let output = work.join("output.json");
    let cleanup = || {
        let _ = std::fs::remove_dir_all(&work);
    };
    std::fs::write(
        &input,
        serde_json::to_string(material).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("材料を書けませんでした: {e}"))?;

    // 変換エンジンの DLL は llama.dll を読むので、常駐と同じく llama_cpu を PATH の先頭に足す（Zenzai は使わない）
    let path = format!(
        "{};{};{}",
        exe_dir.join("llama_cpu").display(),
        exe_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // azookey-server.exe はコンソールのプログラムなので、コンソールを持たない配布版の設定アプリから起動すると
    // 新しいコンソールの窓が開く（閉じると作り直しも止まる）。窓を作らせない
    let mut child = Command::new(&exe)
        .arg("--build-english-vocabulary")
        .arg(&input)
        .arg(&output)
        .current_dir(&exe_dir)
        .env("PATH", path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| {
            cleanup();
            format!(
                "変換エンジンを起動できませんでした（{}）: {e}",
                exe.display()
            )
        })?;
    let mut error = None;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                break;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields.as_slice() {
                ["progress", phase, done, total] => progress(Progress {
                    phase: phase.to_string(),
                    done: done.parse().unwrap_or(0),
                    total: total.parse().unwrap_or(0),
                }),
                ["error", ..] => error = Some(line.trim_start_matches("error ").to_string()),
                _ => {}
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Relaxed) {
        cleanup();
        return Ok(None);
    }
    if !status.success() {
        cleanup();
        return Err(error.unwrap_or_else(|| {
            format!(
                "変換エンジンが途中で終わりました（終了コード {:?}）",
                status.code()
            )
        }));
    }
    let text = std::fs::read_to_string(&output)
        .map_err(|e| format!("作った語彙を読めませんでした: {e}"))?;
    cleanup();
    let vocabulary: VocabularyFile = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    write_atomic(&vocabulary_path(root), &text).map_err(|e| e.to_string())?;
    Ok(Some(vocabulary))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(text: &str) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        count_words(&clean(text), &mut counts);
        counts
    }

    // 段 ① の評価の道具（scripts/english-detection/extract_eval_set.py）と同じ決まりで数える
    #[test]
    fn counts_like_the_evaluation_tool() {
        let counts = counts("claudeとopencodeを使う。You'll see `cargo test` at https://example.com/a and C:\\Soft\\x llama.cpp i2v");
        assert_eq!(counts.get("claude"), Some(&1));
        assert_eq!(counts.get("opencode"), Some(&1));
        assert_eq!(counts.get("you"), None); // you'll は ' でつながる
        assert_eq!(counts.get("ll"), None);
        assert_eq!(counts.get("cargo"), None); // コードの中
        assert_eq!(counts.get("example"), None); // URL の中
        assert_eq!(counts.get("soft"), None); // パスの一部
        assert_eq!(counts.get("llama"), None); // llama.cpp
        assert_eq!(counts.get("see"), Some(&1));
        assert_eq!(counts.get("i"), None); // 1 文字
    }

    #[test]
    fn typed_lines_are_detected_by_prompt_source() {
        assert!(is_typed(
            r#"{"type":"user","promptSource":"typed","message":{}}"#
        ));
        assert!(is_typed(r#"{"type":"user","promptSource":"queued"}"#));
        assert!(!is_typed(r#"{"type":"user","promptSource":"sdk"}"#));
        assert!(!is_typed(r#"{"type":"user"}"#));
    }

    // 覚えた語が先、自分の語彙は回数の多い順。外した語は英字の候補に入れない語へ
    #[test]
    fn word_list_orders_learned_first_and_excludes() {
        let root =
            std::env::temp_dir().join(format!("azookey-english-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let vocabulary = VocabularyFile {
            words: BTreeMap::from([
                ("claude".into(), 1285),
                ("opencode".into(), 207),
                ("kotori".into(), 257),
            ]),
            removed: BTreeMap::from([("made".into(), 1)]),
            built_at: "2026-10-11T09:40:00Z".into(),
        };
        std::fs::write(
            vocabulary_path(&root),
            serde_json::to_string(&vocabulary).unwrap(),
        )
        .unwrap();
        set_learned(&root, "zenzai", Some(Direction::English)).unwrap();
        set_learned(&root, "Kotori", Some(Direction::Japanese)).unwrap();
        let list = word_list(&root);
        let english: Vec<(&str, &str)> = list
            .english
            .iter()
            .map(|r| (r.word.as_str(), r.source))
            .collect();
        assert_eq!(
            english,
            vec![
                ("zenzai", "learned"),
                ("claude", "own"),
                ("opencode", "own")
            ]
        );
        assert_eq!(list.excluded.len(), 1);
        assert_eq!(list.excluded[0].word, "kotori");
        assert_eq!(list.excluded[0].count, 257);
        assert_eq!(list.vocabulary.as_ref().map(|v| v.words), Some(3));
        // 戻すと、自分の語彙の語として英字の候補に戻る
        set_learned(&root, "kotori", None).unwrap();
        assert!(word_list(&root)
            .english
            .iter()
            .any(|r| r.word == "kotori" && r.source == "own"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 実際の会話ログと Vault で、自分の語彙を作り直す処理を通しで走らせる（書き先は一時フォルダ。%APPDATA% には書かない）。
    /// AZOOKEY_SERVER_EXE にビルドした azookey-server.exe を渡す:
    /// `AZOOKEY_SERVER_EXE=<repo>uildzookey-server.exe cargo test --release -p frontend --lib english_real_run -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn english_real_run() {
        let started = Instant::now();
        let files = crate::suggestion::material_files(
            &crate::suggestion::default_claude_projects(),
            &PathBuf::from(
                std::env::var("AZOOKEY_VAULT").unwrap_or_else(|_| r"E:\Obsidian-Vault".to_string()),
            ),
        );
        let cancel = AtomicBool::new(false);
        let material = read_material(&files, &cancel, &mut |_, _| {}).unwrap();
        let read = started.elapsed();
        println!(
            "files {} / words (3+) {} / japanese runs {} / read {:.1}s",
            files.len(),
            material.counts.len(),
            material.japanese.len(),
            read.as_secs_f64()
        );
        let root =
            std::env::temp_dir().join(format!("azookey-english-real-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut phases = Vec::new();
        let vocabulary = build_vocabulary(&material, &root, &cancel, &mut |p| {
            if phases.last() != Some(&p.phase) {
                phases.push(p.phase.clone());
            }
        })
        .unwrap()
        .unwrap();
        println!(
            "phases {:?} / words {} / removed {} / total {:.1}s",
            phases,
            vocabulary.words.len(),
            vocabulary.removed.len(),
            started.elapsed().as_secs_f64()
        );
        assert!(vocabulary_path(&root).exists());
        assert!(vocabulary.words.len() > 1000);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
