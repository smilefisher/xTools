//! 设置：`%APPDATA%\xtools\settings.json`，KV 形式，读失败一律回落默认值。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 全局热键，格式见 `hotkey::parse`（如 "alt+space"）
    pub hotkey: String,
    /// 失焦自动收起
    pub autohide_on_blur: bool,
    pub theme: String,
    /// 文件索引根目录；空表示默认 `%USERPROFILE%`
    pub index_roots: Vec<String>,
    /// 截图热键；默认避开 Alt+Shift（中文系统里是输入法切换键）
    pub shot_hotkey: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "alt+space".into(),
            autohide_on_blur: true,
            theme: "dark".into(),
            index_roots: Vec::new(),
            shot_hotkey: "ctrl+alt+a".into(),
        }
    }
}

static SETTINGS: OnceLock<Mutex<Settings>> = OnceLock::new();

fn cell() -> &'static Mutex<Settings> {
    SETTINGS.get_or_init(|| Mutex::new(Settings::default()))
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    let dir = PathBuf::from(base).join("xtools");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("settings.json"))
}

pub fn init() {
    let loaded = config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
        .unwrap_or_default();

    crate::win::set_autohide_disabled(
        !loaded.autohide_on_blur || std::env::var_os("XTOOLS_NO_AUTOHIDE").is_some(),
    );
    *cell().lock().unwrap() = loaded;
}

pub fn current() -> Settings {
    cell().lock().unwrap().clone()
}

pub fn save(next: Settings) {
    crate::win::set_autohide_disabled(!next.autohide_on_blur);
    if let Some(path) = config_path() {
        if let Ok(raw) = serde_json::to_string_pretty(&next) {
            if let Err(err) = std::fs::write(path, raw) {
                eprintln!("[settings] 写入失败：{err}");
            }
        }
    }
    *cell().lock().unwrap() = next;
}
