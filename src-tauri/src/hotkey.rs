//! 全局热键：默认 Alt+Space，注册失败时由调用方回退到备用组合。

use tauri::AppHandle;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub fn register(app: &AppHandle, spec: &str) -> Result<(), String> {
    register_action(app, spec, |app| crate::win::toggle_palette(app))
}

/// 注册一个自定义动作的热键（如截图）。
pub fn register_action<F>(app: &AppHandle, spec: &str, action: F) -> Result<(), String>
where
    F: Fn(&AppHandle) + Send + Sync + 'static,
{
    let shortcut = parse(spec)
        .ok_or_else(|| format!("无法解析热键 '{spec}'（示例：alt+space / ctrl+shift+space）"))?;

    app.global_shortcut()
        .on_shortcut(shortcut, move |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                action(app);
            }
        })
        .map_err(|e| format!("{e}"))
}

fn parse(spec: &str) -> Option<Shortcut> {
    let mut modifiers = Modifiers::empty();
    let mut code: Option<Code> = None;

    for part in spec.split('+').map(|p| p.trim().to_ascii_lowercase()) {
        match part.as_str() {
            "" => continue,
            "alt" => modifiers |= Modifiers::ALT,
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "shift" => modifiers |= Modifiers::SHIFT,
            "win" | "super" | "meta" => modifiers |= Modifiers::SUPER,
            key => code = Some(map_code(key)?),
        }
    }

    Some(Shortcut::new(Some(modifiers), code?))
}

fn map_code(key: &str) -> Option<Code> {
    Some(match key {
        "space" => Code::Space,
        "enter" | "return" => Code::Enter,
        "tab" => Code::Tab,
        "backquote" | "`" => Code::Backquote,
        "slash" | "/" => Code::Slash,
        "period" | "." => Code::Period,
        "comma" | "," => Code::Comma,
        "semicolon" | ";" => Code::Semicolon,
        "quote" | "'" => Code::Quote,
        "backslash" | "\\" => Code::Backslash,
        "minus" | "-" => Code::Minus,
        "equal" | "=" => Code::Equal,
        "esc" | "escape" => Code::Escape,
        "f1" => Code::F1,
        "f2" => Code::F2,
        "f3" => Code::F3,
        "f4" => Code::F4,
        "f5" => Code::F5,
        "f6" => Code::F6,
        "f7" => Code::F7,
        "f8" => Code::F8,
        "f9" => Code::F9,
        "f10" => Code::F10,
        "f11" => Code::F11,
        "f12" => Code::F12,
        single if single.len() == 1 => {
            let ch = single.chars().next()?;
            match ch {
                'a'..='z' => letter(ch)?,
                '0'..='9' => digit(ch)?,
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn letter(ch: char) -> Option<Code> {
    Some(match ch {
        'a' => Code::KeyA,
        'b' => Code::KeyB,
        'c' => Code::KeyC,
        'd' => Code::KeyD,
        'e' => Code::KeyE,
        'f' => Code::KeyF,
        'g' => Code::KeyG,
        'h' => Code::KeyH,
        'i' => Code::KeyI,
        'j' => Code::KeyJ,
        'k' => Code::KeyK,
        'l' => Code::KeyL,
        'm' => Code::KeyM,
        'n' => Code::KeyN,
        'o' => Code::KeyO,
        'p' => Code::KeyP,
        'q' => Code::KeyQ,
        'r' => Code::KeyR,
        's' => Code::KeyS,
        't' => Code::KeyT,
        'u' => Code::KeyU,
        'v' => Code::KeyV,
        'w' => Code::KeyW,
        'x' => Code::KeyX,
        'y' => Code::KeyY,
        'z' => Code::KeyZ,
        _ => return None,
    })
}

fn digit(ch: char) -> Option<Code> {
    Some(match ch {
        '0' => Code::Digit0,
        '1' => Code::Digit1,
        '2' => Code::Digit2,
        '3' => Code::Digit3,
        '4' => Code::Digit4,
        '5' => Code::Digit5,
        '6' => Code::Digit6,
        '7' => Code::Digit7,
        '8' => Code::Digit8,
        '9' => Code::Digit9,
        _ => return None,
    })
}
