//! 开机自启：写 `HKCU\...\Run`，不需要管理员。

use std::path::PathBuf;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "xTools";

fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

pub fn is_enabled() -> bool {
    let Ok(key) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(RUN_KEY) else {
        return false;
    };
    key.get_value::<String, _>(VALUE_NAME).is_ok()
}

pub fn current_command() -> Option<String> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .ok()?;
    key.get_value::<String, _>(VALUE_NAME).ok()
}

pub fn enable() -> Result<(), String> {
    let exe = exe_path().ok_or("拿不到程序路径")?;
    let quote = format!("\"{}\"", exe.display());
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_ALL_ACCESS)
        .map_err(|e| e.to_string())?;
    key.set_value(VALUE_NAME, &quote).map_err(|e| e.to_string())
}

pub fn disable() -> Result<(), String> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_ALL_ACCESS)
        .map_err(|e| e.to_string())?;
    match key.delete_value(VALUE_NAME) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

pub fn set(enabled: bool) -> Result<(), String> {
    if enabled {
        enable()
    } else {
        disable()
    }
}
