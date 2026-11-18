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
/// 唤起时先抓的冻结帧：提交时从它裁剪，保证不把覆盖层遮罩一起截进去
static FROZEN: OnceLock<Mutex<Option<capture::Frame>>> = OnceLock::new();
static PIN_SEQ: AtomicU32 = AtomicU32::new(1);

fn frozen() -> &'static Mutex<Option<capture::Frame>> {
    FROZEN.get_or_init(|| Mutex::new(None))
}

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

/// 去掉窗口类的 `CS_DROPSHADOW` 样式。
///
/// Tauri 的 `.shadow(false)` 在 Win10 上管不到分层窗口，系统仍会沿窗口类样式
/// 画一圈投影；贴图窗是无边框透明窗，那圈影看起来就像图片自带的阴影。
#[cfg(windows)]
fn kill_drop_shadow(window: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetClassLongPtrW, SetClassLongPtrW, GCL_STYLE};

    const CS_DROPSHADOW: u32 = 0x0002_0000;

    if let Ok(hwnd) = window.hwnd() {
        let handle = HWND(hwnd.0 as *mut core::ffi::c_void);
        unsafe {
            let style = GetClassLongPtrW(handle, GCL_STYLE);
            if style != 0 {
                let cleaned = (style as u32) & !CS_DROPSHADOW;
                if cleaned != style as u32 {
                    SetClassLongPtrW(handle, GCL_STYLE, cleaned as isize);
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn kill_drop_shadow(_window: &tauri::WebviewWindow) {}

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
///
/// 首次创建放到后台线程：热键回调可能在主线程上，直接建窗会自锁。
pub fn begin(app: &AppHandle) -> Option<serde_json::Value> {
    let cursor = app.cursor_position().ok()?;
    let monitor = app.monitor_from_point(cursor.x, cursor.y).ok()??;
    let origin = monitor.position();
    let size = monitor.size();
    let (ox, oy, sw, sh) = (origin.x, origin.y, size.width, size.height);

    if let Ok(mut guard) = last_rect().lock() {
        *guard = Some((origin.x, origin.y, size.width, size.height));
    }

    // 关键：在覆盖层出现之前抓冻结帧，避免把半透明遮罩一起截进图里
    match capture::capture_rect(origin.x, origin.y, size.width as i32, size.height as i32) {
        Ok(frame) => *frozen().lock().unwrap() = Some(frame),
        Err(err) => eprintln!("[shot] 冻结帧抓取失败：{err}"),
    }

    let payload = serde_json::json!({
        "x": origin.x,
        "y": origin.y,
        "width": size.width,
        "height": size.height,
    });

    if let Some(window) = overlay_window(app) {
        let _ = window.set_position(Position::Physical(PhysicalPosition::new(origin.x, origin.y)));
        let _ = window.set_size(Size::Physical(PhysicalSize::new(size.width, size.height)));
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("shot:begin", payload.clone());
        return Some(payload);
    }

    let app = app.clone();
    let emit_payload = payload.clone();
    std::thread::spawn(move || {
        match WebviewWindowBuilder::new(&app, SHOT_LABEL, WebviewUrl::App("index.html#shot".into()))
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
        {
            Ok(window) => {
                let _ = window
                    .set_position(Position::Physical(PhysicalPosition::new(ox, oy)));
                let _ = window.set_size(Size::Physical(PhysicalSize::new(sw, sh)));
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.emit("shot:begin", emit_payload);
            }
            Err(err) => eprintln!("[shot] 创建覆盖窗失败：{err}"),
        }
    });

    Some(payload)
}

/// 诊断用：抓一块屏幕直接贴图，不写剪贴板。
pub fn pin_preview(app: &AppHandle, x: i32, y: i32, width: i32, height: i32) -> Result<u32, String> {
    let frame = capture::capture_rect(x, y, width, height)?;
    create_pin(app, &frame, x, y)
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
    // 覆盖层已隐藏；优先用冻结帧裁剪，拿不到再现场抓（保底）
    let mut frame = {
        let mut guard = frozen().lock().unwrap();
        match guard.take() {
            Some(frozen_frame) => frozen_frame
                .crop(x - frozen_frame.x, y - frozen_frame.y, width as u32, height as u32)
                .ok_or_else(|| "冻结帧裁剪失败".to_string())?,
            None => {
                std::thread::sleep(std::time::Duration::from_millis(80));
                capture::capture_rect(x, y, width, height)?
            }
        }
    };
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
    let pos_x = x as f64 / scale;
    let pos_y = y as f64 / scale;
    let pixel_w = frame.width;
    let pixel_h = frame.height;

    // 贴图窗同样在后台线程创建，避免命令线程把主线程等死
    let app = app.clone();
    std::thread::spawn(move || {
        match WebviewWindowBuilder::new(
            &app,
            &label,
            WebviewUrl::App(format!("index.html#pin/{id}").into()),
        )
        .title("xTools Pin")
        .inner_size(logical_w, logical_h)
        .position(pos_x, pos_y)
        .decorations(false)
        // 贴图是矩形截图，不需要透明；透明分层窗口会被系统附加合成阴影
        .transparent(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .build()
        {
            Ok(window) => {
                kill_drop_shadow(&window);
                // 按物理像素精确对齐，避免 logical→physical 取整后图片被拉伸出一条缝
                let _ = window.set_size(Size::Physical(PhysicalSize::new(pixel_w, pixel_h)));
                let _ = window.set_position(Position::Physical(PhysicalPosition::new(x, y)));
                let _ = window.set_focus();
            }
            Err(err) => eprintln!("[pin] 创建贴图窗失败：{err}"),
        }
    });

    Ok(id)
}

/// 贴图窗用自己的那一段 PNG（ArrayBuffer 回传）。
pub fn pin_bytes(id: u32) -> Option<Vec<u8>> {
    pins().lock().ok()?.get(&id).cloned()
}

/// 缩放贴图窗：按比例改窗口尺寸，内容跟着缩放。
pub fn zoom_pin(app: &AppHandle, id: u32, factor: f64) -> Result<(), String> {
    let window = app
        .get_webview_window(&format!("pin-{id}"))
        .ok_or_else(|| "贴图窗口不存在".to_string())?;

    let size = window.inner_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().unwrap_or(1.0);
    let logical = size.to_logical::<f64>(scale);
    let factor = factor.clamp(0.25, 4.0);

    let width = (logical.width * factor).clamp(48.0, 4000.0);
    let height = (logical.height * factor).clamp(48.0, 4000.0);

    window
        .set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, height)))
        .map_err(|e| e.to_string())
}

/// 把贴图重新塞回剪贴板（右键菜单「复制」）。
pub fn copy_pin(id: u32) -> Result<(), String> {
    let png = pin_bytes(id).ok_or_else(|| "贴图数据不存在".to_string())?;
    let image = image::load_from_memory(&png).map_err(|e| e.to_string())?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();

    let mut bgra = rgba.into_raw();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }

    let frame = capture::Frame {
        x: 0,
        y: 0,
        width,
        height,
        bgra,
    };
    frame.to_clipboard()
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
///
/// 注意：窗口创建必须放到后台线程。同步命令在主线程执行，
/// 而 `WebviewWindowBuilder::build()` 需要回主线程——直接调用会自己等自己卡死。
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

    let app = app.clone();
    std::thread::spawn(move || {
        match WebviewWindowBuilder::new(&app, JSON_LABEL, WebviewUrl::App("index.html#json".into()))
            .title("xTools · JSON")
            .inner_size(960.0, 640.0)
            .min_inner_size(480.0, 320.0)
            .center()
            .build()
        {
            Ok(window) => {
                // 新窗口偶尔抢不到前台，补几次（点击过面板时通常一次就够）
                for delay in [0u64, 120, 320] {
                    if delay > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(delay));
                    }
                    let _ = window.set_focus();
                    let _ = window.unminimize();
                    crate::win::force_foreground(&window);
                }
            }
            Err(err) => eprintln!("[json] 创建窗口失败：{err}"),
        }
    });

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

    // 同 JSON 窗口：必须在后台线程创建，避免主线程自锁
    let app = app.clone();
    std::thread::spawn(move || {
        match WebviewWindowBuilder::new(
            &app,
            SETTINGS_LABEL,
            WebviewUrl::App("index.html#settings".into()),
        )
        .title("xTools · 设置")
        .inner_size(560.0, 640.0)
        .min_inner_size(480.0, 480.0)
        .center()
        .build()
        {
            Ok(window) => {
                let _ = window.set_focus();
            }
            Err(err) => eprintln!("[settings] 创建窗口失败：{err}"),
        }
    });

    Ok(())
}

pub fn close_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = window.close();
    }
}
