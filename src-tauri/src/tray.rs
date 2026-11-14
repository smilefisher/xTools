//! 托盘：常驻入口。左键唤起面板，右键菜单可退出。

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "唤起面板", true, None::<&str>)?;
    let reset = MenuItem::with_id(app, "reset", "重新注册热键", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 xTools", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &PredefinedMenuItem::separator(app)?,
            &reset,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("xtools-tray")
        .tooltip("xTools — Alt+Space 唤起")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => crate::win::show_palette(app),
            "reset" => {
                let spec = crate::settings::current().hotkey;
                match crate::hotkey::register(app, &spec) {
                    Ok(()) => eprintln!("[hotkey] 已重新注册 {spec}"),
                    Err(err) => eprintln!("[hotkey] 重新注册 {spec} 失败：{err}"),
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::win::toggle_palette(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}
