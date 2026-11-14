//! 面板窗口：预热、定位到光标所在屏、抢焦点。
//!
//! 关键约束：窗口在启动时就创建好并隐藏，唤起时只做「显示 + 定位 + 抢焦点」，
//! 绝不重建窗口或重新加载 WebView —— 这是首帧延迟能压到几十毫秒的前提。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Position, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::MAIN_LABEL;

const WINDOW_WIDTH: f64 = 720.0;
const WINDOW_HEIGHT: f64 = 460.0;

/// 手动建窗而不是走 tauri.conf 的窗口声明：为了指定 WebView2 数据目录，
/// 并保证「隐藏创建 + 之后只显示」的预热路径。
pub fn create_main_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let data_dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("xtools")
        .join("WebView2");
    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        eprintln!("[boot] 创建 WebView2 数据目录失败：{err}");
    }

    WebviewWindowBuilder::new(app, MAIN_LABEL, WebviewUrl::App("index.html".into()))
        .title("xTools")
        .inner_size(WINDOW_WIDTH, WINDOW_HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .shadow(false)
        .center()
        .data_directory(data_dir)
        .on_page_load(|_webview, payload| {
            eprintln!("[boot] page load {:?} {}", payload.event(), payload.url());
        })
        .build()
}

static AUTOHIDE_OFF: AtomicBool = AtomicBool::new(false);

pub fn autohide_disabled() -> bool {
    AUTOHIDE_OFF.load(Ordering::Relaxed)
}

pub fn set_autohide_disabled(v: bool) {
    AUTOHIDE_OFF.store(v, Ordering::Relaxed);
}

/// 热键 / 托盘点击的统一入口：可见就收起，不可见就唤起。
pub fn toggle_palette(app: &AppHandle) {
    let Some(win) = app.get_webview_window(MAIN_LABEL) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        show_palette(app);
    }
}

pub fn hide_palette(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(MAIN_LABEL) {
        let _ = win.hide();
    }
}

pub fn show_palette(app: &AppHandle) {
    let Some(win) = app.get_webview_window(MAIN_LABEL) else {
        return;
    };

    // 先定位再显示，避免「在旧位置闪一下再跳过来」
    place_on_cursor_monitor(app, &win);
    let _ = win.show();
    let _ = win.set_focus();
    force_foreground(&win);

    crate::bench::mark_shown();
    let _ = win.emit("palette:shown", ());
}

/// 光标所在显示器上居中偏上，跟随用户当前工作的屏幕。
fn place_on_cursor_monitor(app: &AppHandle, win: &WebviewWindow) {
    let Ok(cursor) = app.cursor_position() else {
        return;
    };
    let Ok(Some(monitor)) = app.monitor_from_point(cursor.x, cursor.y) else {
        return;
    };
    let size = win
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(720, 460));
    let origin = monitor.position();
    let screen = monitor.size();

    let x = origin.x + ((screen.width as i32 - size.width as i32) / 2).max(0);
    let y = origin.y + (screen.height as f64 * 0.16) as i32;

    let _ = win.set_position(Position::Physical(PhysicalPosition::new(x, y)));
}

/// Windows 上 `SetForegroundWindow` 会被前台锁挡住，用 AttachThreadInput 临时并入前台线程。
#[cfg(windows)]
fn force_foreground(win: &WebviewWindow) {
    use std::ffi::c_void;

    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };

    let Ok(raw) = win.hwnd().map(|h| h.0 as isize) else {
        return;
    };
    if raw == 0 {
        return;
    }
    let hwnd = HWND(raw as *mut c_void);

    unsafe {
        let foreground = GetForegroundWindow();
        let fg_thread = GetWindowThreadProcessId(foreground, None);
        let cur_thread = GetCurrentThreadId();
        let mut attached = false;
        if fg_thread != 0 && fg_thread != cur_thread {
            attached = AttachThreadInput(cur_thread, fg_thread, true).as_bool();
        }
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        if attached {
            let _ = AttachThreadInput(cur_thread, fg_thread, false);
        }
    }
}

#[cfg(not(windows))]
fn force_foreground(_win: &WebviewWindow) {}
