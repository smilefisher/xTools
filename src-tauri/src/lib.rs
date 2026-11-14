//! xTools 主程序：常驻托盘 + 全局热键 + 预热面板窗口。

mod bench;
mod capture;
mod cmd;
mod annotate;
mod autostart;
mod service_install;
mod hotkey;
mod json;
mod search;
mod settings;
mod shot;
mod tray;
mod win;

pub const MAIN_LABEL: &str = "main";

use tauri::Manager;

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
                let (mft, dupes) = usn::build_from_records(&format!("{volume}:"), records);
                let index = mft.index;
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

/// `--probe-autostart`：开启 → 读注册表 → 关闭 → 再读，验证自启写入正确。
fn probe_autostart() {
    println!("初始状态：enabled={}", autostart::is_enabled());

    match autostart::enable() {
        Ok(()) => {
            println!("开启后：enabled={}", autostart::is_enabled());
            println!("注册表值：{:?}", autostart::current_command());
        }
        Err(err) => println!("开启失败：{err}"),
    }

    match autostart::disable() {
        Ok(()) => println!("关闭后：enabled={}", autostart::is_enabled()),
        Err(err) => println!("关闭失败：{err}"),
    }
}

/// `--self-check`：不带界面的自检，把环境与各模块状态写成一份报告。
///
/// 用户跑一条命令、把文件给我，就能定位「到底哪一环在环境上不通」。
fn self_check() {
    use std::fmt::Write as _;

    let mut report = String::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let _ = writeln!(report, "xTools 自检报告（unix {now}）");
    let _ = writeln!(report, "版本: {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(report, "程序: {:?}", std::env::current_exe());
    let _ = writeln!(
        report,
        "管理员权限: {}",
        crate::search::app_index::is_elevated_process()
    );
    let _ = writeln!(
        report,
        "索引服务: {}",
        match crate::search::service::ping() {
            Some(entries) => format!("在线（{entries} 条）"),
            None => "未运行".to_string(),
        }
    );
    let _ = writeln!(
        report,
        "计划任务: {}",
        if crate::service_install::task_exists() {
            format!(
                "已安装（{}）",
                crate::service_install::current_run_level().unwrap_or_else(|| "未知".into())
            )
        } else {
            "未安装".to_string()
        }
    );
    let _ = writeln!(
        report,
        "开机自启: {}",
        if crate::autostart::is_enabled() {
            "已启用"
        } else {
            "未启用"
        }
    );

    // 应用索引
    let started = std::time::Instant::now();
    let apps = crate::search::app_index::AppIndex::build();
    let _ = writeln!(
        report,
        "\n[应用索引] {} 条，{:.1} ms",
        apps.entries.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    let usage = std::collections::HashMap::new();
    for query in ["wei", "qq", "code"] {
        let started = std::time::Instant::now();
        let hits = apps.search(query, 3, &usage);
        let _ = writeln!(
            report,
            "  [{query}] {} 条 / {:.3} ms / 首条: {}",
            hits.len(),
            started.elapsed().as_secs_f64() * 1000.0,
            hits.first().map(|h| h.name.clone()).unwrap_or_else(|| "-".into())
        );
    }

    // 文件索引（服务优先，其次本地落盘缓存）
    if let Some(entries) = crate::search::service::ping() {
        let started = std::time::Instant::now();
        let hits = crate::search::service::search("cargo", 5);
        let _ = writeln!(
            report,
            "\n[文件索引] 走服务，{entries} 条；查询 cargo 返回 {} 条 / {:.2} ms",
            hits.as_ref().map(|h| h.len()).unwrap_or(0),
            started.elapsed().as_secs_f64() * 1000.0
        );
    } else {
        let cache = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .map(|base| base.join("xtools").join("index").join("files.bin"));
        match cache.as_deref().and_then(xtools_indexer::persist::load) {
            Some((index, meta)) => {
                let started = std::time::Instant::now();
                let parsed = xtools_indexer::query::Query::parse("cargo");
                let hits = xtools_indexer::query::search(&index, &parsed, 5);
                let _ = writeln!(
                    report,
                    "\n[文件索引] 本地缓存 {} 条（{:.0} 分钟前）；查询 cargo 返回 {} 条 / {:.2} ms",
                    index.len(),
                    meta.age_secs() as f64 / 60.0,
                    hits.len(),
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
            None => {
                let _ = writeln!(report, "\n[文件索引] 无服务、无缓存（首次启动会重建）");
            }
        }
    }

    // JSON
    let sample = r#"{"b":[1,2],"a":"文本"}"#;
    let formatted = crate::json::format(sample, "2", false);
    let tree = crate::json::tree(sample, 100);
    let queried = crate::json::query(sample, "$.b[1]");
    let _ = writeln!(
        report,
        "\n[JSON] 格式化 {} / 树 {} 节点 / 查询 {} / 输出 {}",
        if formatted.ok { "ok" } else { "fail" },
        tree.nodes.len(),
        if queried.ok { &queried.output } else { "fail" },
        formatted.output.replace('\n', " ")
    );

    // 截图
    match crate::capture::capture_rect(0, 0, 120, 80) {
        Ok(frame) => {
            let clip = frame.to_clipboard();
            let _ = writeln!(
                report,
                "\n[截图] 抓屏 ok（{}x{}）；剪贴板 {}",
                frame.width,
                frame.height,
                match clip {
                    Ok(()) => "ok".to_string(),
                    Err(err) => format!("失败（{err}）"),
                }
            );
        }
        Err(err) => {
            let _ = writeln!(report, "\n[截图] 抓屏失败：{err}");
        }
    }

    print!("{report}");

    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        let dir = std::path::PathBuf::from(base).join("xtools").join("logs");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("self-check.txt");
        if std::fs::write(&path, &report).is_ok() {
            println!("报告已写入：{}", path.display());
        }
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

    if std::env::args().any(|arg| arg == "--probe-autostart") {
        probe_autostart();
        return;
    }

    if std::env::args().any(|arg| arg == "--install-index-service") {
        let elevated = crate::search::app_index::is_elevated_process();
        if !elevated {
            println!("安装索引服务需要管理员权限（计划任务写入系统任务库）。");
            println!("请用管理员身份运行，或双击 scripts\\install-service.cmd（会自动申请提权）。");
            return;
        }
        match service_install::install(true) {
            Ok(message) => println!("{message}"),
            Err(err) => println!("安装失败：{err}"),
        }
        println!(
            "任务状态：{}",
            if service_install::task_exists() {
                "已存在"
            } else {
                "不存在"
            }
        );
        return;
    }

    if std::env::args().any(|arg| arg == "--uninstall-index-service") {
        match service_install::uninstall() {
            Ok(message) => println!("{message}"),
            Err(err) => println!("卸载失败：{err}"),
        }
        return;
    }

    if std::env::args().any(|arg| arg == "--self-check") {
        self_check();
        return;
    }

    if std::env::args().any(|arg| arg == "--probe-pipe") {
        match crate::search::service::ping() {
            Some(entries) => {
                println!("索引服务在线：{entries} 条");
                for query in ["cmd", "lib", "main"] {
                    let started = std::time::Instant::now();
                    let hits = crate::search::service::search(query, 3);
                    println!(
                        "[{query}] {:?} 条，{:.2} ms（含连接）",
                        hits.as_ref().map(|h| h.len()),
                        started.elapsed().as_secs_f64() * 1000.0
                    );
                    if let Some(hits) = hits {
                        for hit in hits.iter().take(3) {
                            println!("    {:>4}  {}", hit.score, hit.path);
                        }
                    }
                }
            }
            None => println!("索引服务未运行"),
        }
        return;
    }

    // 设置必须在 AppState 之前加载：索引根目录来自配置
    settings::init();

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
            cmd::settings_open,
            cmd::settings_close,
            cmd::settings_save,
            cmd::autostart_get,
            cmd::autostart_set,
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
            cmd::json_format,
            cmd::json_minify,
            cmd::json_validate,
            cmd::json_query,
            cmd::json_tree,
            cmd::json_open,
            cmd::json_close,
            cmd::json_take_pending,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
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
                    let result = crate::shot::commit(
                        &app,
                        x + 100,
                        y + 100,
                        320,
                        200,
                        true,
                        &[crate::annotate::Op::Rect {
                            x: 20.0,
                            y: 20.0,
                            w: 120.0,
                            h: 80.0,
                            color: Some("#ff0000".into()),
                            width: Some(3.0),
                        }],
                    );
                    eprintln!(
                        "[probe-shot] commit 结果：{result:?}，用时 {:.0} ms",
                        started.elapsed().as_secs_f64() * 1000.0
                    );
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    // 贴图窗缩放自检：读尺寸 → 放大 1.5 倍 → 再读
                    if let Ok(value) = &result {
                        if let Some(id) = value.get("pin").and_then(|v| v.as_u64()) {
                            if let Some(window) = app.get_webview_window(&format!("pin-{id}")) {
                                let before = window.inner_size().ok();
                                let zoomed = crate::shot::zoom_pin(&app, id as u32, 1.5);
                                std::thread::sleep(std::time::Duration::from_millis(300));
                                let after = window.inner_size().ok();
                                eprintln!(
                                    "[probe-shot] 贴图缩放 {before:?} ×1.5 → {after:?}（{zoomed:?}）"
                                );
                            }
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    app.exit(0);
                });
            }

            if std::env::args().any(|arg| arg == "--probe-settings") {
                let app = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1800));
                    match crate::shot::open_settings(&app) {
                        Ok(()) => eprintln!("[probe-settings] 设置窗口已打开"),
                        Err(err) => eprintln!("[probe-settings] 打开失败：{err}"),
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2500));
                    app.exit(0);
                });
            }

            if std::env::args().any(|arg| arg == "--probe-json") {
                let app = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(2000));
                    let sample = r#"{"b":[1,2,{"c":true}],"a":"文本"}"#;
                    let tree = crate::json::tree(sample, 3000);
                    eprintln!(
                        "[probe-json] tree ok={} nodes={} truncated={} {:.2}ms",
                        tree.ok,
                        tree.nodes.len(),
                        tree.truncated,
                        tree.elapsed_ms
                    );
                    match crate::shot::open_json(&app, Some(sample.to_string())) {
                        Ok(()) => eprintln!("[probe-json] 已打开 JSON 窗口"),
                        Err(err) => eprintln!("[probe-json] 打开失败：{err}"),
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2500));
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
