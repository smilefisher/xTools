//! 双击 Shift 唤起：RegisterHotKey 只能表达「修饰键 + 单键」，
//! 连按两次 Shift 这种序列必须用全局低级键盘钩子自己判断。

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

/// 两次 Shift 的最大间隔（毫秒）
const GAP_MS: i64 = 650;

static LAST_SHIFT_MS: AtomicI64 = AtomicI64::new(0);
/// 事件序号：用来确认两次 Shift 之间没有别的键
static SEQ: AtomicU64 = AtomicU64::new(0);
static LAST_SHIFT_SEQ: AtomicU64 = AtomicU64::new(0);
static APP: OnceLock<AppHandle> = OnceLock::new();

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn start(app: AppHandle) {
    if APP.set(app).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("xtools-dblshift".into())
        .spawn(|| unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                CallNextHookEx, GetMessageW, SetWindowsHookExW, MSG, WH_KEYBOARD_LL,
            };

            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0);
            match hook {
                Ok(hook) => {
                    eprintln!("[hotkey] 双击 Shift 监听已启动");
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
                        let _ = windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
                    }
                    let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
                }
                Err(err) => eprintln!("[hotkey] 双击 Shift 钩子安装失败：{err}"),
            }
        })
        .ok();
}

unsafe extern "system" fn hook_proc(code: i32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, KBDLLHOOKSTRUCT, HC_ACTION, WM_KEYDOWN, WM_SYSKEYDOWN,
    };

    if code == HC_ACTION as i32 {
        let event = wparam.0 as u32;
        if event == WM_KEYDOWN || event == WM_SYSKEYDOWN {
            let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let vk = kb.vkCode;
            let is_shift = vk == 0x10 || vk == 0xA0 || vk == 0xA1;
            let ctrl_down = (windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(
                0x11,
            ) as u16
                & 0x8000)
                != 0;
            let alt_down = (windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x12)
                as u16
                & 0x8000)
                != 0;

            let seq = SEQ.fetch_add(1, Ordering::Relaxed) + 1;

            if is_shift && !ctrl_down && !alt_down {
                let previous_seq = LAST_SHIFT_SEQ.load(Ordering::Relaxed);
                let previous_ms = LAST_SHIFT_MS.load(Ordering::Relaxed);
                let now = now_ms();

                // 只有「上一次按键也是 Shift」且间隔够短，才算双击
                if previous_seq != 0 && previous_seq + 1 == seq && now - previous_ms <= GAP_MS {
                    LAST_SHIFT_SEQ.store(0, Ordering::Relaxed);
                    LAST_SHIFT_MS.store(0, Ordering::Relaxed);
                    if let Some(app) = APP.get() {
                        let app = app.clone();
                        std::thread::spawn(move || {
                            eprintln!("[hotkey] 双击 Shift 触发");
                            crate::win::show_palette(&app);
                            // 弹完再确认一次：钩子触发时没有前台权限，
                            // 补一轮抢焦点，避免「弹出来但打不了字」
                            std::thread::sleep(std::time::Duration::from_millis(150));
                            if let Some(window) = app.get_webview_window(crate::MAIN_LABEL) {
                                let focused = window.is_focused().unwrap_or(false);
                                if !focused {
                                    let _ = window.set_focus();
                                    crate::win::force_foreground(&window);
                                    eprintln!("[hotkey] 补抢焦点");
                                }
                            }
                        });
                    }
                } else {
                    if std::env::var_os("XTOOLS_DEBUG").is_some() {
                        eprintln!("[dblshift] shift 按下 seq={seq}");
                    }
                    LAST_SHIFT_SEQ.store(seq, Ordering::Relaxed);
                    LAST_SHIFT_MS.store(now, Ordering::Relaxed);
                }
            } else {
                // 中间夹了别的键，重新计数
                LAST_SHIFT_SEQ.store(0, Ordering::Relaxed);
                LAST_SHIFT_MS.store(0, Ordering::Relaxed);
            }
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}

