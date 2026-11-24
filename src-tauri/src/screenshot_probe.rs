//! 截图辅助：取鼠标下的窗口矩形（对应 PixPin 的 UiSpy），
//! 以及从冻结帧里取一小块像素（放大镜/取色需要）。

use serde::Serialize;

#[derive(Serialize)]
pub struct WindowHit {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub title: String,
    pub class: String,
}

/// 从冻结帧里取一块像素，返回 RGBA。
pub fn frozen_rgba(x: i32, y: i32, w: i32, h: i32) -> Result<Vec<u8>, String> {
    if w <= 0 || h <= 0 {
        return Err("区域无效".into());
    }
    let guard = crate::shot::frozen_frame()
        .lock()
        .map_err(|_| "无法访问冻结帧".to_string())?;
    let frame = guard.as_ref().ok_or_else(|| "没有冻结帧".to_string())?;
    let cropped = frame
        .crop(x - frame.x, y - frame.y, w as u32, h as u32)
        .ok_or_else(|| "裁剪失败".to_string())?;
    Ok(cropped.to_rgba())
}

/// 取屏幕坐标点下方的窗口（走到顶层窗口），排除自身覆盖层与桌面外壳。
#[cfg(windows)]
pub fn window_at(x: i32, y: i32) -> Option<WindowHit> {
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetClassNameW, GetWindowRect, GetWindowTextW, IsWindowVisible,
        WindowFromPoint, GA_ROOT,
    };

    unsafe {
        let mut point = POINT { x, y };
        let hit = WindowFromPoint(point);
        if hit.0.is_null() {
            return None;
        }
        point.x = x;
        let root = GetAncestor(hit, GA_ROOT);
        let target = if root.0.is_null() { hit } else { root };

        if !IsWindowVisible(target).as_bool() {
            return None;
        }

        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(target, &mut class_buf);
        let class = String::from_utf16_lossy(&class_buf[..class_len.max(0) as usize]);

        // 桌面与任务栏不是用户想截的"窗口"
        if matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Tauri Window"
        ) {
            return None;
        }

        let mut rect = windows::Win32::Foundation::RECT::default();
        if GetWindowRect(target, &mut rect).is_err() {
            return None;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width < 8 || height < 8 {
            return None;
        }

        let mut title_buf = [0u16; 512];
        let title_len = GetWindowTextW(target, &mut title_buf);
        let title = String::from_utf16_lossy(&title_buf[..title_len.max(0) as usize]);

        Some(WindowHit {
            x: rect.left,
            y: rect.top,
            width,
            height,
            title,
            class,
        })
    }
}

#[cfg(not(windows))]
pub fn window_at(_x: i32, _y: i32) -> Option<WindowHit> {
    None
}

#[allow(dead_code)]
fn _keep(_: HWNDAlias) {}

type HWNDAlias = isize;
