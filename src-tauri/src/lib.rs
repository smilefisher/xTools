//! xTools 主程序：常驻托盘 + 全局热键 + 预热面板窗口。

mod bench;
mod capture;
mod cmd;
mod hotkey;
mod search;
mod settings;
mod shot;
mod tray;
mod win;

pub const MAIN_LABEL: &str = "main";

/// 开发期调试日志开关（XTOOLS_DEBUG=1）。
pub fn debug_on() -> bool {
    std::env::var_os("XTOOLS_DEBUG").is_some()
}

/// 命令行探针：不开 UI，直接打印索引规模与搜索耗时，用于快速迭代搜索算法。
fn probe_apps() {
    let started = std::time::Instant::now();
    let index = crate::search::app_index::AppIndex::build();
    println!(
        "索引完成：{} 条，耗时 {:.1} ms（含扫描 {:.1} ms）",
        index.entries.len(),
        index.built_ms,
        started.elapsed().as_secs_f64() * 1000.0
    );

    let usage = std::collections::HashMap::new();
    for query in ["wei", "微信", "vs", "code", "calc", "设置", "steam", "qq", "qqyy"] {
        let started = std::time::Instant::now();
        let hits = index.search(query, 5, &usage);
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        println!("\n[{query}]  {elapsed:.3} ms  ({} 条)", hits.len());
        for hit in hits {
            println!("   {:>5}  {}", hit.score, hit.name);
        }
    }

    // 图标提取自检
    if let Some(dir) = crate::search::app_index::icon_cache_dir() {
        let started = std::time::Instant::now();
        let mut ok = 0;
        let mut bytes = 0usize;
        for entry in index.entries.iter().take(10) {
            if let Some(png) = crate::search::app_index::icon_png(&entry.path, &dir) {
                ok += 1;
                bytes += png.len();
            }
        }
        println!(
            "\n图标：{ok}/10 提取成功，共 {} KB，用时 {:.1} ms",
            bytes / 1024,
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// `--probe-files` / `--probe-files=<路径>`：验证文件索引与查询引擎。
fn file_probe_arg() -> Option<String> {
    for arg in std::env::args() {
        if arg == "--probe-files" {
            return Some(
                std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string()),
            );
        }
        if let Some(root) = arg.strip_prefix("--probe-files=") {
            return Some(root.to_string());
        }
    }
    None
}

fn probe_files(root: String) {
    use xtools_indexer::{build_walk, persist, query};

    // 优先复用落盘索引：验证「加载」路径的真实耗时
    let cache = std::env::var_os("LOCALAPPDATA")
        .map(|base| {
            std::path::PathBuf::from(base)
                .join("xtools")
                .join("index")
                .join("files.bin")
        })
        .filter(|path| path.exists());

    let (index, source) = if let Some((index, meta)) = cache.as_deref().and_then(persist::load) {
        (
            index,
            format!(
                "落盘索引（{:.0} 分钟前构建，根 {:?}）",
                meta.age_secs() as f64 / 60.0,
                meta.roots
            ),
        )
    } else {
        let roots = vec![std::path::PathBuf::from(&root)];
        let (index, stats) = build_walk(&roots);
        (
            index,
            format!(
                "现场扫描 {} 条，{:.1}s，内存 {:.1}MB",
                stats.entries,
                stats.elapsed_ms / 1000.0,
                stats.bytes as f64 / 1048576.0
            ),
        )
    };

    println!("来源：{source}");
    println!(
        "索引：{} 条，内存 {:.1} MB",
        index.len(),
        index.memory_bytes() as f64 / 1048576.0
    );

    for text in [
        ".rs",
        "cargo",
        "node_modules",
        "ext:png",
        "config",
        "ext:log build",
    ] {
        let query = query::Query::parse(text);
        let started = std::time::Instant::now();
        let hits = query::search(&index, &query, 5);
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        println!("\n[{text}]  {elapsed:.3} ms  ({} 条)", hits.len());
        for hit in hits {
            println!(
                "   {:>5}  {}{}",
                hit.score,
                if hit.is_dir { "[D] " } else { "" },
                hit.path
            );
        }
    }
}

/// `--probe-mft`：验证 MFT 枚举是否可用（需要管理员），并对比目录扫描。
fn probe_mft() {
    use xtools_indexer::usn;

    println!("管理员权限：{}", usn::is_elevated());
    println!("可索引固定卷：{:?}", usn::fixed_volumes());
    println!("用户目录所在卷：{:?}", xtools_indexer::profile_volume());

    if let Some(volume) = xtools_indexer::profile_volume() {
        let started = std::time::Instant::now();
        match usn::enumerate_volume(volume) {
            Ok(records) => {
                println!(
                    "MFT 枚举 {volume}: 记录 {} 条，用时 {:.2}s",
                    records.len(),
                    started.elapsed().as_secs_f64()
                );
                let (index, dupes) = usn::build_from_records(&format!("{volume}:"), records);
                println!(
                    "组装索引 {} 条（去重 {dupes}），用时 {:.2}s，内存 {:.1}MB",
                    index.len(),
                    started.elapsed().as_secs_f64(),
                    index.memory_bytes() as f64 / 1048576.0
                );
            }
            Err(err) => {
                println!("MFT 枚举失败（预期需要管理员）：{err}");
                println!("→ 实际运行时会自动回退到目录扫描");
            }
        }
    }
}

/// `--probe-capture`：验证抓屏 → 裁剪 → PNG → 剪贴板，无需人工看屏幕。
fn probe_capture() {
    let started = std::time::Instant::now();
    let frame = match capture::capture_rect(0, 0, 800, 600) {
        Ok(frame) => frame,
        Err(err) => {
            println!("抓屏失败：{err}");
            return;
        }
    };
    println!(
        "抓屏 800x600：{:.1} ms，{} KB",
        started.elapsed().as_secs_f64() * 1000.0,
        frame.bgra.len() / 1024
    );

    let started = std::time::Instant::now();
    let cropped = frame.crop(100, 80, 320, 200);
    match &cropped {
        Some(crop) => println!(
            "裁剪 320x200：{:.1} ms，结果 {}x{} @ ({},{})",
            started.elapsed().as_secs_f64() * 1000.0,
            crop.width,
            crop.height,
            crop.x,
            crop.y
        ),
        None => println!("裁剪失败"),
    }

    let started = std::time::Instant::now();
    let path = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join("xtools")
        .join("shots")
        .join("probe.png");
    match &cropped {
        Some(crop) => match crop.save_png(&path) {
            Ok(()) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                println!(
                    "PNG 落盘：{:.1} ms，{} KB → {}",
                    started.elapsed().as_secs_f64() * 1000.0,
                    size / 1024,
                    path.display()
                );
            }
            Err(err) => println!("PNG 落盘失败：{err}"),
        },
        None => {}
    }

    let started = std::time::Instant::now();
    match &cropped {
        Some(crop) => match crop.to_clipboard() {
            Ok(()) => println!("写入剪贴板：{:.1} ms", started.elapsed().as_secs_f64() * 1000.0),
            Err(err) => println!("剪贴板失败：{err}"),
        },
        None => {}
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if std::env::args().any(|arg| arg == "--probe-apps") {
        probe_apps();
        return;
    }

    if let Some(root) = file_probe_arg() {
        probe_files(root);
        return;
    }

    if std::env::args().any(|arg| arg == "--probe-mft") {
        probe_mft();
        return;
    }

    if std::env::args().any(|arg| arg == "--probe-capture") {
        probe_capture();
        return;
    }

    tauri::Builder::default()
        // 单实例必须最先注册：第二次启动直接把已有面板叫出来
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            win::show_palette(app);
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(cmd::AppState::new())
        .invoke_handler(tauri::generate_handler![
            cmd::hide_palette,
            cmd::log_frontend,
            cmd::frontend_ready,
            cmd::first_frame,
            cmd::settings_get,
            cmd::settings_set,
            cmd::app_search,
            cmd::app_launch,
            cmd::app_icon,
            cmd::file_search,
            cmd::file_status,
            cmd::file_open,
            cmd::file_reveal,
            cmd::shot_rect,
            cmd::shot_begin,
            cmd::shot_commit,
            cmd::shot_cancel,
            cmd::pin_image,
            cmd::pin_close,
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

            if let Err(err) = hotkey::register_action(&handle, "alt+shift+a", |app| {
                crate::shot::toggle(app);
            }) {
                eprintln!("[hotkey] 截图热键注册失败：{err}");
            } else {
                eprintln!("[boot] 截图热键 Alt+Shift+A 就绪");
            }

            bench::maybe_start(&handle);
            if let Some(text) = bench::ui_probe_arg() {
                bench::start_ui_probe(&handle, text);
            }
            if std::env::args().any(|arg| arg == "--probe-shot") {
                let app = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(2500));
                    let rect = crate::shot::begin(&app);
                    eprintln!("[probe-shot] 覆盖窗：{rect:?}");
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                    let (x, y) = rect
                        .as_ref()
                        .and_then(|v| {
                            Some((v.get("x")?.as_i64()? as i32, v.get("y")?.as_i64()? as i32))
                        })
                        .unwrap_or((0, 0));
                    let started = std::time::Instant::now();
                    let result = crate::shot::commit(&app, x + 100, y + 100, 320, 200, true);
                    eprintln!(
                        "[probe-shot] commit 结果：{result:?}，用时 {:.0} ms",
                        started.elapsed().as_secs_f64() * 1000.0
                    );
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                    app.exit(0);
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // 关窗不退出，只隐藏：保证窗口与 WebView 永远不重建
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            tauri::WindowEvent::Focused(false) => {
                if window.label() == MAIN_LABEL
                    && !bench::is_running()
                    && !win::autohide_disabled()
                {
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("xTools 启动失败");
}
