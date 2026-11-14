//! 截图覆盖窗与贴图。
//!
//! 流程：热键 → 在光标所在屏铺一个全屏半透明覆盖窗 → 前端拖选 →
//! 回车复制（Rust 侧隐藏窗口后抓屏、写剪贴板）/ Ctrl+回车贴图。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder};

use crate::capture;

pub const SHOT_LABEL: &str = "shot";

static PINS: OnceLock<Mutex<HashMap<u32, Vec<u8>>>> = OnceLock::new();
static LAST_RECT: OnceLock<Mutex<Option<(i32, i32, u32, u32)>>> = OnceLock::new();
static PIN_SEQ: AtomicU32 = AtomicU32::new(1);

fn pins() -> &'static Mutex<HashMap<u32, Vec<u8>>> {
    PINS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn last_rect() -> &'static Mutex<Option<(i32, i32, u32, u32)>> {
    LAST_RECT.get_or_init(|| Mutex::new(None))
}

/// 覆盖窗最近一次覆盖的屏幕矩形（物理像素）。
pub fn last_shot_rect() -> Option<(i32, i32, u32, u32)> {
    *last_rect().lock().ok()?
}

fn overlay_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(SHOT_LABEL)
}

fn ensure_overlay(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(window) = overlay_window(app) {
        return Ok(window);
    }
    WebviewWindowBuilder::new(app, SHOT_LABEL, WebviewUrl::App("index.html#shot".into()))
        .title("xTools Shot")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .visible(false)
        .focused(true)
        .build()
}

/// 热键入口：打开/关闭覆盖窗。
pub fn toggle(app: &AppHandle) {
    if let Some(window) = overlay_window(app) {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
            return;
        }
    }
    begin(app);
}

/// 显示覆盖窗并返回该屏的物理矩形。
pub fn begin(app: &AppHandle) -> Option<serde_json::Value> {
    let Ok(window) = ensure_overlay(app) else {
        eprintln!("[shot] 无法创建覆盖窗");
        return None;
    };

    let cursor = app.cursor_position().ok()?;
    let monitor = app.monitor_from_point(cursor.x, cursor.y).ok()??;
    let origin = monitor.position();
    let size = monitor.size();

    let _ = window.set_position(Position::Physical(PhysicalPosition::new(origin.x, origin.y)));
    let _ = window.set_size(Size::Physical(PhysicalSize::new(size.width, size.height)));
    let _ = window.show();
    let _ = window.set_focus();

    if let Ok(mut guard) = last_rect().lock() {
        *guard = Some((origin.x, origin.y, size.width, size.height));
    }

    let _ = window.emit(
        "shot:begin",
        serde_json::json!({
            "x": origin.x,
            "y": origin.y,
            "width": size.width,
            "height": size.height,
        }),
    );

    Some(serde_json::json!({
        "x": origin.x,
        "y": origin.y,
        "width": size.width,
        "height": size.height,
    }))
}

/// 提交选区：隐藏覆盖窗 → 抓屏 → 剪贴板（+ 可选贴图）。
///
/// 剪贴板写失败（例如锁屏期间系统拒绝访问）不算整体失败：仍然落盘与贴图，
/// 把告警带回前端提示用户。
pub fn commit(
    app: &AppHandle,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    pin: bool,
    ops: &[crate::annotate::Op],
) -> Result<serde_json::Value, String> {
    if width < 2 || height < 2 {
        cancel(app);
        return Err("选区太小".into());
    }
    if let Some(window) = overlay_window(app) {
        let _ = window.hide();
    }
    // 等一帧，确保覆盖窗已经从屏幕上消失
    std::thread::sleep(std::time::Duration::from_millis(80));

    let mut frame = capture::capture_rect(x, y, width, height)?;
    if !ops.is_empty() {
        crate::annotate::apply(&mut frame, ops);
    }
    let frame = frame;

    let mut warnings: Vec<String> = Vec::new();
    let clipboard_ok = match frame.to_clipboard() {
        Ok(()) => true,
        Err(err) => {
            warnings.push(format!("剪贴板写入失败：{err}"));
            false
        }
    };

    let mut saved: Option<String> = None;
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        let path = std::path::PathBuf::from(dir)
            .join("xtools")
            .join("shots")
            .join(format!("shot-{}.png", now_millis()));
        match frame.save_png(&path) {
            Ok(()) => saved = Some(path.to_string_lossy().into_owned()),
            Err(err) => warnings.push(format!("保存失败：{err}")),
        }
    }

    let mut pin_id: Option<u32> = None;
    if pin {
        match create_pin(app, &frame, x, y) {
            Ok(id) => pin_id = Some(id),
            Err(err) => warnings.push(format!("贴图失败：{err}")),
        }
    }

    Ok(serde_json::json!({
        "clipboard": clipboard_ok,
        "saved": saved,
        "pin": pin_id,
        "warnings": warnings,
    }))
}

pub fn cancel(app: &AppHandle) {
    if let Some(window) = overlay_window(app) {
        let _ = window.hide();
    }
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn create_pin(app: &AppHandle, frame: &capture::Frame, x: i32, y: i32) -> Result<u32, String> {
    let id = PIN_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut png = Vec::new();
    {
        let rgba = frame.to_rgba();
        let image = image::RgbaImage::from_raw(frame.width, frame.height, rgba)
            .ok_or_else(|| "贴图尺寸不匹配".to_string())?;
        image
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
    }
    pins().lock().unwrap().insert(id, png);

    let label = format!("pin-{id}");
    let scale = app
        .monitor_from_point(x as f64, y as f64)
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);

    let logical_w = (frame.width as f64 / scale).max(24.0);
    let logical_h = (frame.height as f64 / scale).max(24.0);

    let window = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App(format!("index.html#pin/{id}").into()),
    )
    .title("xTools Pin")
    .inner_size(logical_w, logical_h)
    .position(x as f64 / scale, y as f64 / scale)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .shadow(false)
    .build()
    .map_err(|e| e.to_string())?;

    let _ = window.set_focus();
    Ok(id)
}

/// 贴图窗用自己的那一段 PNG（ArrayBuffer 回传）。
pub fn pin_bytes(id: u32) -> Option<Vec<u8>> {
    pins().lock().ok()?.get(&id).cloned()
}

pub fn close_pin(app: &AppHandle, id: u32) {
    if let Some(window) = app.get_webview_window(&format!("pin-{id}")) {
        let _ = window.close();
    }
    pins().lock().ok().map(|mut map| map.remove(&id));
}

// ---------------------------------------------------------------- JSON 窗口

pub const JSON_LABEL: &str = "json";

static PENDING_JSON: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn pending_json() -> &'static Mutex<Option<String>> {
    PENDING_JSON.get_or_init(|| Mutex::new(None))
}

/// 打开（或聚焦）JSON 工具窗口，可附带一段初始内容。
pub fn open_json(app: &AppHandle, text: Option<String>) -> Result<(), String> {
    if let Some(text) = text.clone() {
        *pending_json().lock().unwrap() = Some(text);
    }

    if let Some(window) = app.get_webview_window(JSON_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        if let Some(text) = text {
            let _ = window.emit("json:set", text);
        }
        return Ok(());
    }

    WebviewWindowBuilder::new(app, JSON_LABEL, WebviewUrl::App("index.html#json".into()))
        .title("xTools · JSON")
        .inner_size(960.0, 640.0)
        .min_inner_size(480.0, 320.0)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 前端挂载后取走待处理内容（首次开窗时事件可能早于监听）。
pub fn take_pending_json() -> Option<String> {
    pending_json().lock().ok()?.take()
}

pub fn close_json(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(JSON_LABEL) {
        let _ = window.hide();
    }
}

// ---------------------------------------------------------------- 设置窗口

pub const SETTINGS_LABEL: &str = "settings";

pub fn open_settings(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    WebviewWindowBuilder::new(
        app,
        SETTINGS_LABEL,
        WebviewUrl::App("index.html#settings".into()),
    )
    .title("xTools · 设置")
    .inner_size(560.0, 640.0)
    .min_inner_size(480.0, 480.0)
    .center()
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn close_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = window.close();
    }
}
