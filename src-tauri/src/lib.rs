//! xTools 主程序：常驻托盘 + 全局热键 + 预热面板窗口。

mod bench;
mod cmd;
mod hotkey;
mod settings;
mod tray;
mod win;

pub const MAIN_LABEL: &str = "main";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例必须最先注册：第二次启动直接把已有面板叫出来
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            win::show_palette(app);
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            cmd::hide_palette,
            cmd::log_frontend,
            cmd::frontend_ready,
            cmd::first_frame,
            cmd::settings_get,
            cmd::settings_set,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            settings::init();
            eprintln!("[boot] settings ok, hotkey={}", settings::current().hotkey);

            // 预热：窗口与 WebView 一次创建，之后只做显示/隐藏
            match win::create_main_window(&handle) {
                Ok(_) => eprintln!("[boot] 预热窗口就绪"),
                Err(err) => eprintln!("[boot] 窗口创建失败：{err}"),
            }

            // 托盘失败不致命：面板与热键仍可用
            if let Err(err) = tray::create(&handle) {
                eprintln!("[boot] 托盘创建失败：{err}");
            } else {
                eprintln!("[boot] tray ok");
            }

            let preferred = settings::current().hotkey;
            if let Err(err) = hotkey::register(&handle, &preferred) {
                eprintln!("[hotkey] 注册 {preferred} 失败：{err}");
                // 备用组合：被输入法/其它工具抢占时仍可用
                const FALLBACKS: [&str; 3] = ["ctrl+space", "alt+shift+space", "ctrl+alt+space"];
                for spec in FALLBACKS {
                    if hotkey::register(&handle, spec).is_ok() {
                        eprintln!("[hotkey] 已回退到 {spec}");
                        break;
                    }
                }
            }

            bench::maybe_start(&handle);
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // 关窗不退出，只隐藏：保证窗口与 WebView 永远不重建
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            tauri::WindowEvent::Focused(false) => {
                if !bench::is_running() && !win::autohide_disabled() {
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("xTools 启动失败");
}
