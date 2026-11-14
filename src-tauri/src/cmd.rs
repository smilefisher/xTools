//! 暴露给前端的命令。前端只做渲染与输入，窗口/热键等动作一律回落 Rust。

use tauri::AppHandle;

use crate::settings::{self, Settings};

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
