//! 首帧延迟测量。
//!
//! 打点方式：唤起时在 Rust 侧记录 `Instant`，前端在第一次 rAF 后回调 `first_frame`，
//! 两者之差即「热键 → 首帧」的真实耗时。`--bench[=N]` 会自驱 N 轮并输出 p50/p95。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static RUNNING: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);
static SHOWN_AT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
static SAMPLES: OnceLock<Mutex<Vec<f64>>> = OnceLock::new();

/// 前端挂载完成（事件监听已就绪），bench 在此之前不会开始计时，避免空跑。
pub fn mark_ready() {
    READY.store(true, Ordering::Relaxed);
}

pub fn is_ready() -> bool {
    READY.load(Ordering::Relaxed)
}

fn shown_at() -> &'static Mutex<Option<Instant>> {
    SHOWN_AT.get_or_init(|| Mutex::new(None))
}

fn samples() -> &'static Mutex<Vec<f64>> {
    SAMPLES.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn mark_shown() {
    *shown_at().lock().unwrap() = Some(Instant::now());
}

/// 前端首帧回调。返回本次耗时（毫秒），没有待测样本时返回 None。
pub fn record_first_frame() -> Option<f64> {
    let taken = shown_at().lock().unwrap().take()?;
    let ms = taken.elapsed().as_secs_f64() * 1000.0;
    samples().lock().unwrap().push(ms);
    Some(ms)
}

pub fn is_running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

fn bench_arg() -> Option<usize> {
    for arg in std::env::args() {
        if arg == "--bench" {
            return Some(10);
        }
        if let Some(v) = arg.strip_prefix("--bench=") {
            return v.parse().ok();
        }
    }
    None
}

/// `--probe-ui=<关键词>`：唤起面板并让前端跑一次真实查询，用于无人工介入的 UI 冒烟测试。
pub fn ui_probe_arg() -> Option<String> {
    std::env::args().find_map(|arg| {
        arg.strip_prefix("--probe-ui=")
            .map(|value| value.to_string())
    })
}

pub fn start_ui_probe(app: &tauri::AppHandle, text: String) {
    let app = app.clone();
    std::thread::spawn(move || {
        let ready_deadline = Instant::now() + Duration::from_secs(20);
        while !is_ready() && Instant::now() < ready_deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(300));

        crate::win::show_palette(&app);
        std::thread::sleep(Duration::from_millis(600));

        let _ = tauri::Emitter::emit(&app, "ui:query", text.clone());
        eprintln!("[probe] 已发送查询 {text:?}");

        std::thread::sleep(Duration::from_millis(2500));
        app.exit(0);
    });
}

pub fn maybe_start(app: &tauri::AppHandle) {
    let Some(count) = bench_arg() else {
        return;
    };
    RUNNING.store(true, Ordering::Relaxed);

    let app = app.clone();
    std::thread::spawn(move || {
        // 等前端挂载完成，否则测到的是冷加载而不是唤起延迟
        let ready_deadline = Instant::now() + Duration::from_secs(20);
        while !is_ready() && Instant::now() < ready_deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(300));

        for _ in 0..count {
            let seen = samples().lock().unwrap().len();
            crate::win::show_palette(&app);

            let deadline = Instant::now() + Duration::from_secs(3);
            while samples().lock().unwrap().len() == seen && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
            std::thread::sleep(Duration::from_millis(120));
            crate::win::hide_palette(&app);
            std::thread::sleep(Duration::from_millis(220));
        }

        report_and_exit(&app);
    });
}

fn report_and_exit(app: &tauri::AppHandle) {
    let mut values = samples().lock().unwrap().clone();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let quantile = |q: f64| -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        let idx = ((values.len() - 1) as f64 * q).round() as usize;
        values[idx]
    };

    let report = serde_json::json!({
        "samples": values.len(),
        "min_ms": quantile(0.0),
        "p50_ms": quantile(0.5),
        "p95_ms": quantile(0.95),
        "max_ms": quantile(1.0),
        "all_ms": values,
    });

    println!("BENCH {report}");
    let _ = std::fs::write("bench-latency.json", report.to_string());

    RUNNING.store(false, Ordering::Relaxed);
    app.exit(0);
}
