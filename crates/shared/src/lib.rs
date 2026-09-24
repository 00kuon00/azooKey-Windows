use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/azookey.rs"));
    include!(concat!(env!("OUT_DIR"), "/window.rs"));
    pub const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("azookey_service_descriptor");
}

fn get_config_root() -> PathBuf {
    let appdata = PathBuf::from(std::env::var("APPDATA").unwrap());
    appdata.join("Azookey")
}

const SETTINGS_FILENAME: &str = "settings.json";

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ZenzaiConfig {
    pub enable: bool,
    pub profile: String,
    pub backend: String,
}

// 学習の設定。mode は変換エンジンの LearningType と同じ名前
// （inputAndOutput = 学習する / onlyOutput = 新しく学習しない / nothing = 学習しない）
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LearningConfig {
    pub mode: String,
}

impl Default for LearningConfig {
    fn default() -> Self {
        LearningConfig {
            mode: "inputAndOutput".to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppConfig {
    pub version: String,
    pub zenzai: ZenzaiConfig,
    // 学習の設定が無い古い settings.json も読めるようにする
    #[serde(default)]
    pub learning: LearningConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            version: "0.1.0".to_string(),
            zenzai: ZenzaiConfig {
                enable: false,
                profile: "".to_string(),
                backend: "cpu".to_string(),
            },
            learning: LearningConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn write(&self) {
        let config_path = get_config_root().join(SETTINGS_FILENAME);
        let config_str = serde_json::to_string_pretty(self).unwrap();
        std::fs::write(config_path, config_str).unwrap();
    }

    pub fn read() -> Self {
        let config_path = get_config_root().join(SETTINGS_FILENAME);
        if !config_path.exists() {
            return AppConfig::default();
        }
        let config_str = std::fs::read_to_string(config_path).unwrap();
        serde_json::from_str(&config_str).unwrap()
    }

    pub fn new() -> Self {
        let config_path = get_config_root();
        if !config_path.exists() {
            std::fs::create_dir_all(&config_path).unwrap();
        }
        let config = AppConfig::read();
        config.write();
        config
    }
}

const USER_DICTIONARY_FILENAME: &str = "user_dictionary.json";

/// ユーザー辞書の 1 語。元データは settings.json に混ぜず、%APPDATA%\Azookey\user_dictionary.json に配列で置く。
/// 変換用の辞書（user.louds など）は変換エンジン（server-swift）がこれから作る
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct UserDictionaryEntry {
    pub reading: String,
    pub word: String,
}

impl UserDictionaryEntry {
    pub fn read_all() -> std::io::Result<Vec<UserDictionaryEntry>> {
        let path = get_config_root().join(USER_DICTIONARY_FILENAME);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// 一時ファイルに書いてから置き換える（途中で落ちても元のファイルを壊さない）
    pub fn write_all(entries: &[UserDictionaryEntry]) -> std::io::Result<()> {
        let root = get_config_root();
        std::fs::create_dir_all(&root)?;
        let path = root.join(USER_DICTIONARY_FILENAME);
        let temp = root.join(format!("{USER_DICTIONARY_FILENAME}.tmp"));
        let text = serde_json::to_string_pretty(entries)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, &path)
    }
}
