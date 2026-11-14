//! 索引助手进程：建索引 + 命名管道只读服务。
//!
//! 有管理员权限时走 MFT（秒级），否则退回目录扫描（分钟级），两者对外协议一致。
//! 安全边界：只接受「查询」请求，不接受任何执行/打开路径的指令。

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde_json::{json, Value};
use xtools_indexer::query::{self, Query};
use xtools_indexer::{build_walk, usn, Index};

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    for item in args {
        if let Some(value) = item.strip_prefix(&prefix) {
            return Some(value.to_string());
        }
    }
    None
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pipe = arg_value(&args, "--pipe").unwrap_or_else(|| "xtools-index-v1".to_string());
    let roots: Vec<PathBuf> = match arg_value(&args, "--roots") {
        Some(raw) => std::env::split_paths(&raw).collect(),
        None => default_roots(),
    };

    println!("[indexd] 管道 {pipe}，根：{:?}", roots);
    let index = build_index(&roots);
    println!("[indexd] 索引就绪：{} 条", index.len());

    let index = Arc::new(RwLock::new(index));

    if let Err(err) = xtools_pipe::serve(&pipe, move |request| {
        handle(request, &index)
    }) {
        eprintln!("[indexd] 管道服务退出：{err}");
    }
}

fn default_roots() -> Vec<PathBuf> {
    match std::env::var_os("USERPROFILE") {
        Some(profile) => vec![PathBuf::from(profile)],
        None => vec![PathBuf::from("C:\\")],
    }
}

fn build_index(roots: &[PathBuf]) -> Index {
    if usn::is_elevated() {
        if let Some(volume) = xtools_indexer::profile_volume() {
            match usn::enumerate_volume(volume) {
                Ok(records) => {
                    if usn::looks_sane(&records) {
                        let started = std::time::Instant::now();
                        let (mft, duplicates) =
                            usn::build_from_records(&format!("{volume}:"), records);
                        println!(
                            "[indexd] MFT 索引 {} 条（去重 {duplicates}），用时 {:.1}s",
                            mft.index.len(),
                            started.elapsed().as_secs_f64()
                        );
                        return mft.index;
                    }
                    eprintln!("[indexd] MFT 结果异常，回退目录扫描");
                }
                Err(err) => eprintln!("[indexd] MFT 枚举失败（{err}），回退目录扫描"),
            }
        }
    }

    let started = std::time::Instant::now();
    let (index, stats) = build_walk(roots);
    println!(
        "[indexd] 目录扫描 {} 条，用时 {:.1}s，内存 {:.1}MB",
        stats.entries,
        started.elapsed().as_secs_f64(),
        stats.bytes as f64 / 1048576.0
    );
    index
}

fn handle(request: &str, index: &Arc<RwLock<Index>>) -> String {
    let started = std::time::Instant::now();
    let parsed: Value = match serde_json::from_str(request) {
        Ok(value) => value,
        Err(err) => return json!({"ok": false, "error": err.to_string()}).to_string(),
    };

    let op = parsed.get("op").and_then(|v| v.as_str()).unwrap_or("search");

    match op {
        "ping" => json!({
            "ok": true,
            "entries": index.read().map(|i| i.len()).unwrap_or(0),
            "pid": std::process::id(),
        })
        .to_string(),
        "search" => {
            let text = parsed.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let limit = parsed.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let parsed_query = Query::parse(text);

            let guard = match index.read() {
                Ok(guard) => guard,
                Err(_) => return json!({"ok": false, "error": "索引锁不可用"}).to_string(),
            };

            let hits = query::search(&guard, &parsed_query, limit);
            json!({
                "ok": true,
                "hits": hits,
                "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
            })
            .to_string()
        }
        other => json!({"ok": false, "error": format!("未知操作 {other}")}).to_string(),
    }
}
