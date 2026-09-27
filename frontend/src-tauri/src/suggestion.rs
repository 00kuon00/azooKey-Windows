//! ユーザー辞書の登録候補（#17）。
//! Claude Code の会話ログのユーザー発言と Obsidian Vault のノートから、よく使うのに変換で出にくい語を探す。
//! 流れ: 材料を読んで語を数える → 変換エンジンで「登録しなくても上位に出る語」を落とす →
//! 残りの読みをローカル LLM に付けさせる → 付けた読みでもう一度変換エンジンに調べさせる。
//! 材料は読むだけで書き換えない。辞書への登録は画面で承認されたものだけ（ここでは登録しない）

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// 読みを付ける LLM（4090 機に常駐している llama-swap・OpenAI 互換。台帳 apps.yaml の llama-vision）
pub const LLM_ENDPOINT: &str = "http://192.168.1.184:8092";
/// 候補に残すのに要る出現回数
pub const MIN_COUNT: usize = 3;
/// 変換エンジンに調べさせる語の数の上限（出現回数の多い順）
pub const MAX_CHECKED_TERMS: usize = 3000;
/// LLM に読みを付けさせる語の数の上限（出現回数の多い順）
pub const MAX_LLM_TERMS: usize = 150;
/// 変換エンジンへの 1 回の問い合わせで送る語の数。
/// 変換エンジンは IME の変換と同じスレッドで調べるので、その間の入力が待たされない程度に小さくする
pub const ENGINE_BATCH: usize = 8;
/// LLM への 1 回の問い合わせで送る語の数
pub const LLM_BATCH: usize = 10;
/// 1 語に付けておく用例の数
pub const MAX_EXAMPLES: usize = 3;
/// 用例に語の前後から含める文字数
const EXAMPLE_CONTEXT_CHARS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Claude Code の会話ログ（ユーザーの発言）
    Conversation,
    /// Obsidian Vault のノート
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKind {
    Katakana,
    Kanji,
    Alphabet,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Example {
    pub source: Source,
    pub text: String,
}

/// 拾い出した語と、その出現回数・用例
#[derive(Debug, Clone, PartialEq)]
pub struct Term {
    pub word: String,
    pub kind: TermKind,
    pub count: usize,
    pub examples: Vec<Example>,
}

/// 画面に出す候補
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub word: String,
    /// 読み（ひらがな）。LLM がひらがなで返せなかったときはそのまま入れ、画面で直させる
    pub reading: String,
    pub count: usize,
    pub examples: Vec<Example>,
}

static KATAKANA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ァ-ヺ][ァ-ヺー]{2,}").unwrap());
static KANJI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{Han}々〆ヶ]{2,8}").unwrap());
static ALPHABET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z][A-Za-z0-9]*(?:[+\-][A-Za-z0-9]+)*").unwrap());
// ユーザーが書いていない部分（Claude Code が差し込んだもの・貼り付けたもの）
static INJECTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)<system-reminder>.*?</system-reminder>|<pasted_content[^>]*>.*?</pasted_content>",
    )
    .unwrap()
});

/// 英字の語のうち、名前らしいもの（大文字を含む 3 文字以上）だけを拾う。
/// 小文字だけの語はコマンドやコードの断片が多く、日本語の文の中でも英字入力で打つので除く。
/// 前後に英数字・パスの区切り・拡張子が続くもの（`C:\Soft\...` の `Soft`・`AGENTS.md`）も除く
fn is_name_like(text: &str, start: usize, end: usize) -> bool {
    let word = &text[start..end];
    if word.chars().count() < 3 || !word.chars().any(|c| c.is_ascii_uppercase()) {
        return false;
    }
    let before = text[..start].chars().next_back();
    let mut after = text[end..].chars();
    let joins = |c: char| {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '\\' | '-' | '@' | '$' | '%')
    };
    // 「.」は後ろに英数字が続くとき（拡張子・ドメイン）だけつながっているとみなす。英文の文末の「Claude.」は拾う
    let joined_after = match after.next() {
        Some('.') => after.next().is_some_and(|c| c.is_ascii_alphanumeric()),
        Some(c) => joins(c),
        None => false,
    };
    !before.is_some_and(|c| joins(c) || c == '.') && !joined_after
}

/// 漢字の並びのすぐ後ろに送り仮名らしいひらがなが続くか。
/// 「箇条書き」の「箇条書」のように語の途中で切れたものを数えないよう、
/// 後ろが助詞・助動詞の頭（と、する動詞の「し・す・さ・せ」）以外のひらがなのときは拾わない
fn ends_before_okurigana(text: &str, end: usize) -> bool {
    match text[end..].chars().next() {
        Some(c) if ('\u{3041}'..='\u{3096}').contains(&c) => {
            !"のをにはがでともへやかだなしすさせ".contains(c)
        }
        _ => false,
    }
}

/// 文から語を拾う。返り値は（語, 種類, 文の中のバイト位置）
pub fn extract_terms(text: &str) -> Vec<(String, TermKind, usize)> {
    let mut terms = Vec::new();
    for found in KATAKANA.find_iter(text) {
        terms.push((
            found.as_str().to_string(),
            TermKind::Katakana,
            found.start(),
        ));
    }
    for found in KANJI.find_iter(text) {
        if !ends_before_okurigana(text, found.end()) {
            terms.push((found.as_str().to_string(), TermKind::Kanji, found.start()));
        }
    }
    for found in ALPHABET.find_iter(text) {
        if is_name_like(text, found.start(), found.end()) {
            terms.push((
                found.as_str().to_string(),
                TermKind::Alphabet,
                found.start(),
            ));
        }
    }
    terms
}

/// 語の前後を少し含めた用例。改行と連続した空白は 1 つの空白にする
fn example_around(text: &str, start: usize, word: &str) -> String {
    let before: Vec<char> = text[..start]
        .chars()
        .rev()
        .take(EXAMPLE_CONTEXT_CHARS)
        .collect();
    let after: String = text[start + word.len()..]
        .chars()
        .take(EXAMPLE_CONTEXT_CHARS)
        .collect();
    let before: String = before.into_iter().rev().collect();
    // 改行をまたいだ前後は別の話のことが多いので切る
    let before = before.rsplit('\n').next().unwrap_or_default();
    let after = after.split('\n').next().unwrap_or_default();
    let joined = format!("{before}{word}{after}");
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 語を数える。同じ文の中に何度出ても、用例は 1 つしか取らない
#[derive(Debug, Default)]
pub struct TermCounter {
    terms: HashMap<String, Term>,
    pub texts: usize,
}

impl TermCounter {
    pub fn add_text(&mut self, text: &str, source: Source) {
        self.texts += 1;
        let mut seen = HashSet::new();
        for (word, kind, start) in extract_terms(text) {
            let term = self.terms.entry(word.clone()).or_insert_with(|| Term {
                word: word.clone(),
                kind,
                count: 0,
                examples: Vec::new(),
            });
            term.count += 1;
            if term.examples.len() < MAX_EXAMPLES && seen.insert(word.clone()) {
                let example = example_around(text, start, &word);
                if !term.examples.iter().any(|e| e.text == example) {
                    term.examples.push(Example {
                        source,
                        text: example,
                    });
                }
            }
        }
    }

    /// 候補にする語を出現回数の多い順に返す。
    /// 回数が `MIN_COUNT` に満たない語・ユーザー辞書にある語・却下した語は除く
    pub fn candidates(
        &self,
        registered: &HashSet<String>,
        rejected: &HashSet<String>,
    ) -> Vec<Term> {
        let mut terms: Vec<Term> = self
            .terms
            .values()
            .filter(|term| term.count >= MIN_COUNT)
            .filter(|term| !registered.contains(&term.word) && !rejected.contains(&term.word))
            .cloned()
            .collect();
        terms.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.word.cmp(&b.word)));
        terms
    }
}

/// 会話ログの 1 行（JSON）から、ユーザーが書いた文を取り出す。
/// ツールの結果・Claude Code が差し込んだ文（スキルの本文・コマンド・通知）・サブエージェントの会話は除く
pub fn user_texts_from_log_line(line: &str) -> Vec<String> {
    // 4GB 近くあるので、JSON として読む前に文字列で振り分ける
    if !line.contains("\"type\":\"user\"") || line.contains("\"type\":\"tool_result\"") {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Vec::new();
    };
    let is_true = |key: &str| value.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    if value.get("type").and_then(|v| v.as_str()) != Some("user")
        || is_true("isMeta")
        || is_true("isSidechain")
    {
        return Vec::new();
    }
    let parts: Vec<String> = match value.pointer("/message/content") {
        Some(serde_json::Value::String(text)) => vec![text.clone()],
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(|v| v.as_str()) == Some("text"))
            .filter_map(|block| {
                block
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect(),
        _ => Vec::new(),
    };
    parts
        .into_iter()
        .map(|text| INJECTED.replace_all(&text, "").trim().to_string())
        .filter(|text| {
            !text.is_empty()
                && !text.starts_with('<')
                && !text.starts_with("Base directory for this skill")
        })
        .collect()
}

/// 材料のファイル。会話ログは `projects/*/*.jsonl`（サブエージェントの `subagents/` は含めない）、
/// ノートは Vault の Knowledge・Projects・Daily の下の `*.md`
pub fn material_files(claude_projects: &Path, vault: &Path) -> Vec<(PathBuf, Source)> {
    let mut files = Vec::new();
    if let Ok(projects) = std::fs::read_dir(claude_projects) {
        for project in projects.flatten() {
            if let Ok(entries) = std::fs::read_dir(project.path()) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() && path.extension().is_some_and(|e| e == "jsonl") {
                        files.push((path, Source::Conversation));
                    }
                }
            }
        }
    }
    fn walk(dir: &Path, files: &mut Vec<(PathBuf, Source)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|e| e == "md") {
                files.push((path, Source::Note));
            }
        }
    }
    for folder in ["Knowledge", "Projects", "Daily"] {
        walk(&vault.join(folder), &mut files);
    }
    files
}

pub fn default_claude_projects() -> PathBuf {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    PathBuf::from(home).join(".claude").join("projects")
}

pub fn default_vault() -> PathBuf {
    PathBuf::from(r"E:\Obsidian-Vault")
}

/// 読みとして使える文字（ひらがなと長音）だけか
pub fn is_hiragana_reading(reading: &str) -> bool {
    !reading.is_empty()
        && reading
            .chars()
            .all(|c| ('\u{3041}'..='\u{3096}').contains(&c) || c == 'ー')
}

/// カタカナをひらがなにし、空白を除く（LLM がカタカナで返すことがある）
pub fn normalize_reading(reading: &str) -> String {
    reading
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '\u{30A1}'..='\u{30F6}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// 変換エンジンの答え
#[derive(Debug, Clone, PartialEq)]
pub struct Convertibility {
    pub word: String,
    /// 調べるのに使った読み（推定できなければ空）
    pub reading: String,
    pub convertible: bool,
}

/// 変換エンジン（語が登録しなくても上位に出るかを調べる）。本番は IPC、テストでは差し替える
pub trait Engine {
    /// `queries` は（語, 読み）。読みが空なら変換エンジンが辞書から推定する
    fn check(&mut self, queries: &[(String, String)]) -> Result<Vec<Convertibility>, String>;
}

/// 読みを付ける LLM。本番は `LlmClient`、テストでは差し替える
pub trait ReadingProvider {
    /// 語ごとの読み。登録する意味が無い語は空。返さなかった語は空とみなす
    fn readings(&mut self, terms: &[Term]) -> Result<HashMap<String, String>, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Connect,
    Read,
    Check,
    Reading,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
    /// ここまでに見つかった候補の数
    pub found: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ScanStats {
    /// 読んだ材料（会話の発言とノート）の数
    pub texts: usize,
    /// 変換エンジンに調べさせた語の数
    pub checked: usize,
    /// LLM に読みを付けさせた語の数
    pub llm: usize,
    pub seconds: f64,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    /// 停止ボタンで途中で止めたか
    pub stopped: bool,
    pub suggestions: Vec<Suggestion>,
    pub stats: ScanStats,
}

/// 語から、登録しなくても変換できるものを落とす。返り値は残った語と、変換エンジンが推定した読み（空なら推定できなかった）
pub fn drop_convertible(
    terms: Vec<Term>,
    readings: &HashMap<String, String>,
    engine: &mut dyn Engine,
    cancel: &AtomicBool,
    mut on_batch: impl FnMut(usize),
) -> Result<(Vec<(Term, String)>, bool), String> {
    let mut remaining = Vec::new();
    for batch in terms.chunks(ENGINE_BATCH) {
        if cancel.load(Ordering::Relaxed) {
            return Ok((remaining, true));
        }
        let queries: Vec<(String, String)> = batch
            .iter()
            .map(|term| {
                (
                    term.word.clone(),
                    readings.get(&term.word).cloned().unwrap_or_default(),
                )
            })
            .collect();
        let results = engine.check(&queries)?;
        for term in batch {
            // 答えが欠けた語は変換できないものとして残す（黙って捨てない）
            match results.iter().find(|r| r.word == term.word) {
                Some(result) if result.convertible => {}
                Some(result) => remaining.push((term.clone(), result.reading.clone())),
                None => remaining.push((term.clone(), String::new())),
            }
        }
        on_batch(batch.len());
    }
    Ok((remaining, false))
}

/// 候補を探す本体。材料の文は `texts` で渡す（読み込みと分けてテストできるように）
pub fn find_suggestions(
    counter: &TermCounter,
    registered: &HashSet<String>,
    rejected: &HashSet<String>,
    engine: &mut dyn Engine,
    llm: &mut dyn ReadingProvider,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<(Vec<Suggestion>, ScanStats, bool), String> {
    let mut stats = ScanStats {
        texts: counter.texts,
        ..Default::default()
    };
    let mut suggestions: Vec<Suggestion> = Vec::new();
    let to_suggestion = |term: &Term, reading: String| Suggestion {
        word: term.word.clone(),
        reading,
        count: term.count,
        examples: term.examples.clone(),
    };

    // 1. 変換エンジンが推定した読みで上位に出る語を落とす
    let terms: Vec<Term> = counter
        .candidates(registered, rejected)
        .into_iter()
        .take(MAX_CHECKED_TERMS)
        .collect();
    let total = terms.len() as u64;
    stats.checked = terms.len();
    let mut done = 0u64;
    let (remaining, stopped) = drop_convertible(terms, &HashMap::new(), engine, cancel, |n| {
        done += n as u64;
        progress(Progress {
            phase: Phase::Check,
            done,
            total,
            found: 0,
        });
    })?;
    if stopped {
        return Ok((suggestions, stats, true));
    }

    // 2. カタカナの語は読みが決まっているので、LLM に聞かずに候補にする。
    //    漢字・英字の語は LLM に読みを付けさせる（辞書から推定した読みでは出なかったので、読みが違う見込みが高い）
    let mut ask = Vec::new();
    for (term, reading) in remaining {
        if term.kind == TermKind::Katakana && is_hiragana_reading(&reading) {
            suggestions.push(to_suggestion(&term, reading));
        } else {
            ask.push(term);
        }
    }
    ask.truncate(MAX_LLM_TERMS);
    stats.llm = ask.len();

    // 3. LLM が付けた読みでもう一度調べ、それでも上位に出ない語を候補にする
    let total = ask.len() as u64;
    let mut done = 0u64;
    for batch in ask.chunks(LLM_BATCH) {
        if cancel.load(Ordering::Relaxed) {
            return Ok((suggestions, stats, true));
        }
        let readings: HashMap<String, String> = llm
            .readings(batch)?
            .into_iter()
            .map(|(word, reading)| (word, normalize_reading(&reading)))
            .filter(|(_, reading)| !reading.is_empty())
            .collect();
        // ひらがなでない読みは変換エンジンに渡せないので、調べずに候補に残して画面で直させる
        let (valid, invalid): (Vec<Term>, Vec<Term>) = batch
            .iter()
            .filter(|term| readings.contains_key(&term.word))
            .cloned()
            .partition(|term| is_hiragana_reading(&readings[&term.word]));
        for term in &invalid {
            suggestions.push(to_suggestion(term, readings[&term.word].clone()));
        }
        let (remaining, stopped) = drop_convertible(valid, &readings, engine, cancel, |_| {})?;
        for (term, _) in remaining {
            let reading = readings[&term.word].clone();
            suggestions.push(to_suggestion(&term, reading));
        }
        done += batch.len() as u64;
        progress(Progress {
            phase: Phase::Reading,
            done,
            total,
            found: suggestions.len(),
        });
        if stopped {
            return Ok((suggestions, stats, true));
        }
    }
    suggestions.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.word.cmp(&b.word)));
    Ok((suggestions, stats, false))
}

/// 材料を読んで語を数える。進み具合はバイト数で出す（会話ログはファイルごとの大きさの差が大きい）
pub fn read_materials(
    files: &[(PathBuf, Source)],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> (TermCounter, bool) {
    let mut counter = TermCounter::default();
    let total: u64 = files
        .iter()
        .filter_map(|(path, _)| path.metadata().ok())
        .map(|m| m.len())
        .sum();
    let mut done = 0u64;
    let mut last_report = Instant::now();
    for (path, source) in files {
        if cancel.load(Ordering::Relaxed) {
            return (counter, true);
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
                    for text in user_texts_from_log_line(&String::from_utf8_lossy(&line)) {
                        counter.add_text(&text, Source::Conversation);
                    }
                    if last_report.elapsed() > Duration::from_millis(100) {
                        if cancel.load(Ordering::Relaxed) {
                            return (counter, true);
                        }
                        progress(Progress {
                            phase: Phase::Read,
                            done,
                            total,
                            found: 0,
                        });
                        last_report = Instant::now();
                    }
                }
            }
            Source::Note => {
                let mut bytes = Vec::new();
                use std::io::Read;
                if BufReader::new(file).read_to_end(&mut bytes).is_ok() {
                    done += bytes.len() as u64;
                    counter.add_text(&String::from_utf8_lossy(&bytes), Source::Note);
                }
            }
        }
    }
    progress(Progress {
        phase: Phase::Read,
        done: total,
        total,
        found: 0,
    });
    (counter, false)
}

/// 4090 機の LLM（llama-swap・OpenAI 互換）
pub struct LlmClient {
    client: reqwest::blocking::Client,
    pub model: String,
}

const LLM_INSTRUCTIONS: &str = "あなたは日本語の IME のユーザー辞書を作る手伝いをします。与えられた語それぞれについて、日本語の文を打つときにその語を出すために入力する読みを、ひらがなで答えてください。
- 英字の名前は、日本語で話すときの呼び方をひらがなにします。英字の略語は 1 文字ずつ読みます。大文字で区切られた複合語（CamelCase）は、つなげて読みます
- 例: GitHub → ぎっとはぶ / YouTube → ゆーちゅーぶ / PyTorch → ぱいとーち / GPU → じーぴーゆー / WebUI → うぇぶゆーあい / OpenStreetMap → おーぷんすとりーとまっぷ / 東雲 → しののめ
- 長音は「ー」。カタカナは使わない
- 一般的な英単語（The・Request など）・コードの断片・ファイルパスの一部・ログの文字列（INFO など）は、辞書に登録しても役に立たないので reading を空文字にします
- example はその語が使われていた文です。読みの手がかりにしてください";

impl LlmClient {
    /// 届くかを確かめ、使うモデルを決める。
    /// llama-swap は指定したモデルに入れ替えるので、いま動いているモデル（/running）を使い、他のアプリのモデルを追い出さない。
    /// 動いているものが無いときだけ、一覧の先頭を使う
    pub fn connect() -> Result<Self, String> {
        let unreachable =
            |e: reqwest::Error| format!("4090 機の LLM（{LLM_ENDPOINT}）に届きませんでした: {e}");
        let probe = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| e.to_string())?;
        let running: serde_json::Value = probe
            .get(format!("{LLM_ENDPOINT}/running"))
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
            .map_err(unreachable)?;
        let model = match running.pointer("/running/0/model").and_then(|v| v.as_str()) {
            Some(model) => model.to_string(),
            None => {
                let models: serde_json::Value = probe
                    .get(format!("{LLM_ENDPOINT}/v1/models"))
                    .send()
                    .and_then(|r| r.error_for_status())
                    .and_then(|r| r.json())
                    .map_err(unreachable)?;
                models
                    .pointer("/data/0/id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("4090 機の LLM（{LLM_ENDPOINT}）にモデルがありません"))?
                    .to_string()
            }
        };
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client, model })
    }
}

#[derive(Deserialize)]
struct LlmItems {
    items: Vec<LlmItem>,
}

#[derive(Deserialize)]
struct LlmItem {
    word: String,
    reading: String,
}

/// LLM の答え（`{"items": [{"word": ..., "reading": ...}]}`）を読む
pub fn parse_llm_readings(content: &str) -> Result<HashMap<String, String>, String> {
    let parsed: LlmItems =
        serde_json::from_str(content).map_err(|e| format!("LLM の答えを読めませんでした: {e}"))?;
    Ok(parsed
        .items
        .into_iter()
        .map(|item| (item.word, item.reading))
        .collect())
}

impl ReadingProvider for LlmClient {
    fn readings(&mut self, terms: &[Term]) -> Result<HashMap<String, String>, String> {
        let input = terms
            .iter()
            .map(|term| {
                let example = term
                    .examples
                    .first()
                    .map(|e| e.text.as_str())
                    .unwrap_or_default();
                serde_json::json!({ "word": term.word, "example": example }).to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": LLM_INSTRUCTIONS },
                { "role": "user", "content": input },
            ],
            "temperature": 0,
            // 考える過程を出させない（出すと答えが遅れ、本文が空になることがある）
            "chat_template_kwargs": { "enable_thinking": false },
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "readings",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "items": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": { "word": { "type": "string" }, "reading": { "type": "string" } },
                                    "required": ["word", "reading"],
                                },
                            },
                        },
                        "required": ["items"],
                    },
                },
            },
        });
        let response: serde_json::Value = self
            .client
            .post(format!("{LLM_ENDPOINT}/v1/chat/completions"))
            .json(&body)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
            .map_err(|e| {
                format!("4090 機の LLM（{LLM_ENDPOINT}）への問い合わせに失敗しました: {e}")
            })?;
        let content = response
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        parse_llm_readings(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        extract_terms(text)
            .into_iter()
            .map(|(word, _, _)| word)
            .collect()
    }

    #[test]
    fn extracts_katakana_kanji_and_names() {
        let found = words("ComfyUI を起動してシャニマスの立ち絵を生成する");
        assert!(found.contains(&"ComfyUI".to_string()));
        assert!(found.contains(&"シャニマス".to_string()));
        assert!(found.contains(&"起動".to_string()));
        assert!(found.contains(&"生成".to_string()));
    }

    #[test]
    fn skips_lowercase_words_paths_and_filenames() {
        let found = words(
            r"git status を見て C:\Soft\DiaryCompanion と AGENTS.md と ~/.claude/Projects を開く",
        );
        assert!(!found.contains(&"git".to_string()));
        assert!(!found.contains(&"Soft".to_string()));
        assert!(!found.contains(&"DiaryCompanion".to_string()));
        assert!(!found.contains(&"AGENTS".to_string()));
        assert!(!found.contains(&"Projects".to_string()));
    }

    #[test]
    fn kanji_cut_before_okurigana_is_not_a_term() {
        let found = words("箇条書きで書く。実装した画面の設定を見る");
        assert!(!found.contains(&"箇条書".to_string()));
        assert!(found.contains(&"実装".to_string()));
        assert!(found.contains(&"画面".to_string()));
        assert!(found.contains(&"設定".to_string()));
    }

    #[test]
    fn name_at_end_of_english_sentence_is_kept() {
        assert_eq!(words("I asked Claude."), vec!["Claude"]);
        assert!(words("see README.md").is_empty());
    }

    #[test]
    fn short_katakana_and_single_kanji_are_not_terms() {
        assert!(words("ドア を 開 く").is_empty());
    }

    #[test]
    fn counts_and_keeps_examples_per_text() {
        let mut counter = TermCounter::default();
        counter.add_text("Kotori の語彙と Kotori の辞書", Source::Conversation);
        counter.add_text("Kotori を使う", Source::Note);
        counter.add_text("Kotori を使う", Source::Note);
        let terms = counter.candidates(&HashSet::new(), &HashSet::new());
        let kotori = terms.iter().find(|t| t.word == "Kotori").unwrap();
        assert_eq!(kotori.count, 4);
        // 同じ文の用例は 1 つにまとめる
        assert_eq!(
            kotori.examples,
            vec![
                Example {
                    source: Source::Conversation,
                    text: "Kotori の語彙と Kotori の辞書".into()
                },
                Example {
                    source: Source::Note,
                    text: "Kotori を使う".into()
                },
            ]
        );
        assert_eq!(counter.texts, 3);
    }

    #[test]
    fn example_is_cut_at_newlines_and_length() {
        let text = format!(
            "前の段落\n{}ComfyUI を{}\n次の段落",
            "あ".repeat(40),
            "い".repeat(40)
        );
        let start = text.find("ComfyUI").unwrap();
        let example = example_around(&text, start, "ComfyUI");
        assert_eq!(
            example,
            format!(
                "{}ComfyUI {}",
                "あ".repeat(24),
                "を".to_string() + &"い".repeat(22)
            )
        );
    }

    #[test]
    fn candidates_are_sorted_and_exclude_rare_registered_and_rejected() {
        let mut counter = TermCounter::default();
        for _ in 0..5 {
            counter.add_text("ComfyUI", Source::Note);
        }
        for _ in 0..4 {
            counter.add_text("Kotori と MiniMax と Zenzai", Source::Note);
        }
        counter.add_text("Phantrope Phantrope", Source::Note);
        let registered = HashSet::from(["Zenzai".to_string()]);
        let rejected = HashSet::from(["MiniMax".to_string()]);
        let order: Vec<String> = counter
            .candidates(&registered, &rejected)
            .into_iter()
            .map(|t| t.word)
            .collect();
        assert_eq!(order, vec!["ComfyUI", "Kotori"]);
    }

    #[test]
    fn reads_only_what_the_user_wrote() {
        let user = r#"{"type":"user","message":{"role":"user","content":"ComfyUI を起動して<system-reminder>Claude の注意書き</system-reminder>"}}"#;
        assert_eq!(user_texts_from_log_line(user), vec!["ComfyUI を起動して"]);

        let blocks = r#"{"type":"user","message":{"content":[{"type":"text","text":"Kotori の辞書"},{"type":"image","source":{}}]}}"#;
        assert_eq!(user_texts_from_log_line(blocks), vec!["Kotori の辞書"]);

        let excluded = [
            r#"{"type":"assistant","message":{"content":"ComfyUI"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ComfyUI"}]}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"ComfyUI"}}"#,
            r#"{"type":"user","isSidechain":true,"message":{"content":"ComfyUI"}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"Base directory for this skill: C:\\skills"}]}}"#,
            r#"{"type":"user","message":{"content":"<pasted_content id=\"1\">貼り付けた文</pasted_content>"}}"#,
            "壊れた行",
        ];
        for line in excluded {
            assert!(user_texts_from_log_line(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn normalizes_and_validates_readings() {
        assert_eq!(
            normalize_reading("コンフィー ユーアイ"),
            "こんふぃーゆーあい"
        );
        assert!(is_hiragana_reading("ぶいらむ"));
        assert!(!is_hiragana_reading("たいき中"));
        assert!(!is_hiragana_reading(""));
        assert!(!is_hiragana_reading("ぺर्सなる"));
    }

    #[test]
    fn parses_llm_answer() {
        let readings = parse_llm_readings(
            r#"{"items":[{"word":"Kotori","reading":"ことり"},{"word":"The","reading":""}]}"#,
        )
        .unwrap();
        assert_eq!(readings["Kotori"], "ことり");
        assert_eq!(readings["The"], "");
        assert!(parse_llm_readings("ことり").is_err());
    }

    /// 決めた語だけ「上位に出る」と答える変換エンジン
    struct FakeEngine {
        /// 読みを渡さなかったとき（辞書から推定したとき）に変換できる語と、その読み
        known: HashMap<&'static str, &'static str>,
        /// （語, 読み）の組で変換できるもの
        convertible_with: HashSet<(&'static str, &'static str)>,
        calls: Vec<Vec<(String, String)>>,
    }

    impl Engine for FakeEngine {
        fn check(&mut self, queries: &[(String, String)]) -> Result<Vec<Convertibility>, String> {
            self.calls.push(queries.to_vec());
            Ok(queries
                .iter()
                .map(|(word, reading)| {
                    if reading.is_empty() {
                        let known = self.known.get(word.as_str());
                        let inferred = if word
                            .chars()
                            .all(|c| ('ァ'..='ヺ').contains(&c) || c == 'ー')
                        {
                            normalize_reading(word)
                        } else {
                            known.map(|r| r.to_string()).unwrap_or_default()
                        };
                        Convertibility {
                            word: word.clone(),
                            reading: inferred,
                            convertible: known.is_some(),
                        }
                    } else {
                        let convertible = self
                            .convertible_with
                            .iter()
                            .any(|(w, r)| w == word && r == reading);
                        Convertibility {
                            word: word.clone(),
                            reading: reading.clone(),
                            convertible,
                        }
                    }
                })
                .collect())
        }
    }

    struct FakeLlm {
        answers: HashMap<&'static str, &'static str>,
        asked: Vec<String>,
    }

    impl ReadingProvider for FakeLlm {
        fn readings(&mut self, terms: &[Term]) -> Result<HashMap<String, String>, String> {
            self.asked.extend(terms.iter().map(|t| t.word.clone()));
            Ok(terms
                .iter()
                .filter_map(|t| {
                    self.answers
                        .get(t.word.as_str())
                        .map(|r| (t.word.clone(), r.to_string()))
                })
                .collect())
        }
    }

    fn counter_of(words: &[(&str, usize)]) -> TermCounter {
        let mut counter = TermCounter::default();
        for (word, count) in words {
            for _ in 0..*count {
                counter.add_text(word, Source::Conversation);
            }
        }
        counter
    }

    #[test]
    fn drops_words_that_convert_without_registering() {
        let counter = counter_of(&[
            ("東京", 9),       // 辞書の読みで上位に出る → 落とす
            ("ファイル", 8),   // カタカナも上位に出る → 落とす
            ("シャニマス", 7), // カタカナで上位に出ない → LLM に聞かずに候補
            ("ComfyUI", 6),    // LLM の読みでも上位に出ない → 候補
            ("VRAM", 5),       // LLM の読みで上位に出る → 落とす
            ("Request", 4),    // LLM が「登録する意味が無い」と答える → 落とす
            ("待機中", 3),     // LLM の読みがひらがなでない → 直させるため候補に残す
        ]);
        let mut engine = FakeEngine {
            known: HashMap::from([("東京", "とうきょう"), ("ファイル", "ふぁいる")]),
            convertible_with: HashSet::from([("VRAM", "ぶいらむ")]),
            calls: Vec::new(),
        };
        let mut llm = FakeLlm {
            answers: HashMap::from([
                ("ComfyUI", "コンフィーユーアイ"),
                ("VRAM", "ぶいらむ"),
                ("Request", ""),
                ("待機中", "たいき中"),
            ]),
            asked: Vec::new(),
        };
        let cancel = AtomicBool::new(false);
        let (suggestions, stats, stopped) = find_suggestions(
            &counter,
            &HashSet::new(),
            &HashSet::new(),
            &mut engine,
            &mut llm,
            &cancel,
            &mut |_| {},
        )
        .unwrap();
        assert!(!stopped);
        let got: Vec<(&str, &str, usize)> = suggestions
            .iter()
            .map(|s| (s.word.as_str(), s.reading.as_str(), s.count))
            .collect();
        assert_eq!(
            got,
            vec![
                ("シャニマス", "しゃにます", 7),
                ("ComfyUI", "こんふぃーゆーあい", 6),
                ("待機中", "たいき中", 3)
            ]
        );
        // カタカナの語は LLM に聞かない
        assert_eq!(llm.asked, vec!["ComfyUI", "VRAM", "Request", "待機中"]);
        assert_eq!((stats.checked, stats.llm, stats.texts), (7, 4, 42));
    }

    #[test]
    fn stops_when_cancelled() {
        let counter = counter_of(&[("ComfyUI", 3)]);
        let mut engine = FakeEngine {
            known: HashMap::new(),
            convertible_with: HashSet::new(),
            calls: Vec::new(),
        };
        let mut llm = FakeLlm {
            answers: HashMap::new(),
            asked: Vec::new(),
        };
        let cancel = AtomicBool::new(true);
        let (suggestions, _, stopped) = find_suggestions(
            &counter,
            &HashSet::new(),
            &HashSet::new(),
            &mut engine,
            &mut llm,
            &cancel,
            &mut |_| {},
        )
        .unwrap();
        assert!(stopped);
        assert!(suggestions.is_empty());
        assert!(engine.calls.is_empty());
    }

    /// 実際の材料で 1 回走らせる（辞書には登録しない）。新しい RPC を持つ変換エンジンを別名のパイプで立て、
    /// AZOOKEY_PIPE_NAME にその名前、AZOOKEY_REAL_RUN_OUT に結果の JSON の置き場を渡す:
    /// `cargo test --release -p frontend --lib real_run -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_run() {
        struct Timed {
            inner: crate::IpcEngine,
            calls: u32,
            total: Duration,
            longest: Duration,
        }
        impl Engine for Timed {
            fn check(
                &mut self,
                queries: &[(String, String)],
            ) -> Result<Vec<Convertibility>, String> {
                let started = Instant::now();
                let result = self.inner.check(queries);
                let elapsed = started.elapsed();
                self.calls += 1;
                self.total += elapsed;
                self.longest = self.longest.max(elapsed);
                result
            }
        }
        let started = Instant::now();
        let cancel = AtomicBool::new(false);
        let mut llm = LlmClient::connect().unwrap();
        let registered: HashSet<String> = shared::UserDictionaryEntry::read_all()
            .unwrap()
            .into_iter()
            .map(|e| e.word)
            .collect();
        let rejected: HashSet<String> = shared::read_rejected_suggestions()
            .unwrap()
            .into_iter()
            .collect();
        let files = material_files(&default_claude_projects(), &default_vault());
        let (counter, _) = read_materials(&files, &cancel, &mut |_| {});
        let read_seconds = started.elapsed().as_secs_f64();
        let mut engine = Timed {
            inner: crate::IpcEngine(crate::ipc::IPCService::new().unwrap()),
            calls: 0,
            total: Duration::ZERO,
            longest: Duration::ZERO,
        };
        let mut phases = Vec::new();
        let (suggestions, stats, stopped) = find_suggestions(
            &counter,
            &registered,
            &rejected,
            &mut engine,
            &mut llm,
            &cancel,
            &mut |p| {
                if p.done == p.total {
                    phases.push((p.phase, started.elapsed().as_secs_f64()));
                }
            },
        )
        .unwrap();
        let report = serde_json::json!({
            "seconds": started.elapsed().as_secs_f64(),
            "read_seconds": read_seconds,
            "phase_end_seconds": phases.iter().map(|(p, s)| serde_json::json!([p, s])).collect::<Vec<_>>(),
            "files": files.len(),
            "texts": stats.texts,
            "checked": stats.checked,
            "llm": stats.llm,
            "model": llm.model,
            "stopped": stopped,
            "engine_calls": engine.calls,
            "engine_average_ms": engine.total.as_secs_f64() * 1000.0 / engine.calls.max(1) as f64,
            "engine_longest_ms": engine.longest.as_secs_f64() * 1000.0,
            "count": suggestions.len(),
            "suggestions": suggestions,
        });
        let out = std::env::var("AZOOKEY_REAL_RUN_OUT").unwrap();
        std::fs::write(out, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }

    #[test]
    fn engine_is_asked_in_small_batches() {
        let words: Vec<(String, usize)> = (0..40).map(|i| (format!("Name{i:02}"), 3)).collect();
        let refs: Vec<(&str, usize)> = words.iter().map(|(w, c)| (w.as_str(), *c)).collect();
        let counter = counter_of(&refs);
        let mut engine = FakeEngine {
            known: HashMap::new(),
            convertible_with: HashSet::new(),
            calls: Vec::new(),
        };
        let mut llm = FakeLlm {
            answers: HashMap::new(),
            asked: Vec::new(),
        };
        let cancel = AtomicBool::new(false);
        find_suggestions(
            &counter,
            &HashSet::new(),
            &HashSet::new(),
            &mut engine,
            &mut llm,
            &cancel,
            &mut |_| {},
        )
        .unwrap();
        assert!(engine.calls.iter().all(|call| call.len() <= ENGINE_BATCH));
        assert_eq!(engine.calls.iter().map(Vec::len).sum::<usize>(), 40);
    }
}
