//! 暴露给前端的命令。前端只做渲染与输入，窗口/热键等动作一律回落 Rust。

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, State};

use crate::search::app_index::{AppHit, AppIndex, UsageStore};
use crate::search::file_index::FileState;
use crate::settings::{self, Settings};

pub struct AppState {
    pub index: Mutex<AppIndex>,
    pub usage: Mutex<UsageStore>,
    pub files: Arc<FileState>,
}

impl AppState {
    pub fn new() -> Self {
        let index = AppIndex::build();
        eprintln!(
            "[index] 应用索引 {} 条，用时 {:.1} ms",
            index.entries.len(),
            index.built_ms
        );

        let files = Arc::new(FileState::new());
        files.start();

        Self {
            index: Mutex::new(index),
            usage: Mutex::new(UsageStore::load()),
            files,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[tauri::command]
pub fn hide_palette(app: AppHandle) {
    crate::win::hide_palette(&app);
}

/// 前端调试日志：开发期把关键事件打到 stderr，方便在没有 devtools 时定位问题。
#[tauri::command]
pub fn log_frontend(message: String) {
    eprintln!("[fe] {message}");
}

/// 前端已挂载、事件监听就绪。
#[tauri::command]
pub fn frontend_ready() {
    crate::bench::mark_ready();
    eprintln!("[fe] ready");
}

/// 前端首帧回调，返回「唤起 → 首帧」耗时（毫秒）。
#[tauri::command]
pub fn first_frame() -> Option<f64> {
    let ms = crate::bench::record_first_frame();
    if let Some(ms) = ms {
        eprintln!("[bench] hotkey -> first frame: {ms:.2} ms");
    }
    ms
}

#[tauri::command]
pub fn settings_get() -> Settings {
    settings::current()
}

#[tauri::command]
pub fn settings_set(next: Settings) -> Settings {
    settings::save(next);
    settings::current()
}

// ---------------------------------------------------------------- 设置窗口

#[tauri::command]
pub fn settings_open(app: AppHandle) -> Result<(), String> {
    crate::shot::open_settings(&app)
}

#[tauri::command]
pub fn settings_close(app: AppHandle) {
    crate::shot::close_settings(&app);
}

#[tauri::command]
pub fn settings_save(app: AppHandle, next: Settings) -> Result<serde_json::Value, String> {
    settings::save(next);
    let problems = crate::hotkey::reregister_all(&app);
    Ok(serde_json::json!({
        "settings": settings::current(),
        "problems": problems,
    }))
}

#[tauri::command]
pub fn autostart_get() -> serde_json::Value {
    serde_json::json!({
        "enabled": crate::autostart::is_enabled(),
        "command": crate::autostart::current_command(),
        "exe": std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
    })
}

#[tauri::command]
pub fn autostart_set(enabled: bool) -> Result<bool, String> {
    crate::autostart::set(enabled)?;
    Ok(crate::autostart::is_enabled())
}

// ---------------------------------------------------------------- 应用搜索

#[tauri::command]
pub fn app_search(state: State<'_, AppState>, query: String, limit: Option<usize>) -> Vec<AppHit> {
    let started = std::time::Instant::now();
    let index = state.index.lock().unwrap();
    let usage = state.usage.lock().unwrap();
    let hits = index.search(&query, limit.unwrap_or(50), &usage.apps);
    if crate::debug_on() {
        eprintln!(
            "[app] query={query:?} hits={} {:.3}ms",
            hits.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    hits
}

#[tauri::command]
pub fn app_launch(state: State<'_, AppState>, id: u32) -> Result<(), String> {
    let path = {
        let index = state.index.lock().unwrap();
        index.find(id).map(|entry| entry.path.clone())
    };
    let Some(path) = path else {
        return Err("找不到该应用，索引可能已过期".into());
    };
    state.usage.lock().unwrap().record(id);
    crate::search::app_index::launch(&path, false)
}

/// 图标以 ArrayBuffer 回传，前端直接转 blob URL，避免 base64 膨胀。
#[tauri::command]
pub fn app_icon(state: State<'_, AppState>, id: u32) -> tauri::ipc::Response {
    let path = {
        let index = state.index.lock().unwrap();
        index.find(id).map(|entry| entry.path.clone())
    };
    let png = path
        .and_then(|path| {
            crate::search::app_index::icon_cache_dir()
                .and_then(|dir| crate::search::app_index::icon_png(&path, &dir))
        })
        .unwrap_or_default();
    tauri::ipc::Response::new(png)
}

// ---------------------------------------------------------------- 文件搜索

#[tauri::command]
pub fn file_search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Vec<xtools_indexer::query::Hit> {
    let started = std::time::Instant::now();
    let hits = state.files.search(&query, limit.unwrap_or(20));
    if crate::debug_on() {
        eprintln!(
            "[file] query={query:?} hits={} {:.3}ms",
            hits.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    hits
}

#[tauri::command]
pub fn file_status(state: State<'_, AppState>) -> serde_json::Value {
    serde_json::json!({
        "ready": state.files.is_ready(),
        "entries": state.files.entry_count(),
    })
}

#[tauri::command]
pub fn file_open(path: String) -> Result<(), String> {
    crate::search::app_index::launch(&path, false)
}

#[tauri::command]
pub fn file_reveal(path: String) -> Result<(), String> {
    crate::search::file_index::reveal(&path)
}

// ---------------------------------------------------------------- 截图 / 贴图

#[tauri::command]
pub fn shot_rect() -> serde_json::Value {
    match crate::shot::last_shot_rect() {
        Some((x, y, width, height)) => serde_json::json!({
            "x": x, "y": y, "width": width, "height": height
        }),
        None => serde_json::Value::Null,
    }
}

#[tauri::command]
pub fn shot_begin(app: AppHandle) -> serde_json::Value {
    crate::shot::begin(&app).unwrap_or(serde_json::Value::Null)
}

#[tauri::command]
pub fn shot_commit(
    app: AppHandle,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    pin: Option<bool>,
    ops: Option<Vec<crate::annotate::Op>>,
) -> Result<serde_json::Value, String> {
    crate::shot::commit(
        &app,
        x,
        y,
        width,
        height,
        pin.unwrap_or(false),
        &ops.unwrap_or_default(),
    )
}

#[tauri::command]
pub fn shot_cancel(app: AppHandle) {
    crate::shot::cancel(&app);
}

#[tauri::command]
pub fn pin_image(id: u32) -> tauri::ipc::Response {
    tauri::ipc::Response::new(crate::shot::pin_bytes(id).unwrap_or_default())
}

#[tauri::command]
pub fn pin_close(app: AppHandle, id: u32) {
    crate::shot::close_pin(&app, id);
}

// ---------------------------------------------------------------- JSON 工具

#[tauri::command]
pub fn json_format(
    text: String,
    indent: Option<String>,
    sort_keys: Option<bool>,
) -> crate::json::JsonResult {
    crate::json::format(&text, indent.as_deref().unwrap_or("2"), sort_keys.unwrap_or(false))
}

#[tauri::command]
pub fn json_minify(text: String) -> crate::json::JsonResult {
    crate::json::minify(&text)
}

#[tauri::command]
pub fn json_validate(text: String) -> crate::json::JsonResult {
    crate::json::validate(&text)
}

#[tauri::command]
pub fn json_query(text: String, path: String) -> crate::json::JsonResult {
    crate::json::query(&text, &path)
}

#[tauri::command]
pub fn json_tree(text: String, limit: Option<usize>) -> crate::json::JsonTree {
    crate::json::tree(&text, limit.unwrap_or(3000))
}

#[tauri::command]
pub fn json_open(app: AppHandle, text: Option<String>) -> Result<(), String> {
    crate::shot::open_json(&app, text)
}

#[tauri::command]
pub fn json_close(app: AppHandle) {
    crate::shot::close_json(&app);
}

#[tauri::command]
pub fn json_take_pending() -> Option<String> {
    crate::shot::take_pending_json()
}
