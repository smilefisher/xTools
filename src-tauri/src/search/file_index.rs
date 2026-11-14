//! 文件索引状态与后台构建。
//!
//! 当前版本用目录遍历建索引（无需管理员），落盘后下次启动直接加载。
//! MFT/USN 的提权索引服务作为下一阶段接入，接口保持不变。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use xtools_indexer::query::{self, Hit};
use xtools_indexer::{build_walk, persist, Index};

const MAX_AGE_SECS: u64 = 6 * 3600;

pub struct FileState {
    index: RwLock<Option<Index>>,
    ready: AtomicBool,
    roots: Vec<PathBuf>,
}

impl FileState {
    pub fn new() -> Self {
        let roots = default_roots();
        Self {
            index: RwLock::new(None),
            ready: AtomicBool::new(false),
            roots,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    pub fn entry_count(&self) -> usize {
        self.index
            .read()
            .ok()
            .and_then(|guard| guard.as_ref().map(|index| index.len()))
            .unwrap_or(0)
    }

    /// 后台线程：优先加载落盘索引，过期或缺失才重新扫描。
    pub fn start(self: &Arc<Self>) {
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name("xtools-file-index".into())
            .spawn(move || me.load_or_build())
            .ok();
    }

    fn load_or_build(&self) {
        let cache = index_path();
        let root_labels: Vec<String> = self
            .roots
            .iter()
            .map(|r| r.to_string_lossy().into_owned())
            .collect();

        if let Some((index, meta)) = cache.as_deref().and_then(persist::load) {
            let fresh = meta.age_secs() < MAX_AGE_SECS && meta.roots == root_labels;
            if fresh {
                eprintln!(
                    "[file] 载入索引 {} 条（{:.0} 分钟前构建）",
                    index.len(),
                    meta.age_secs() as f64 / 60.0
                );
                if let Ok(mut guard) = self.index.write() {
                    *guard = Some(index);
                }
                self.ready.store(true, Ordering::Relaxed);
                return;
            }
        }

        let (index, stats) = build_walk(&self.roots);
        eprintln!(
            "[file] 构建索引 {} 条，用时 {:.1}s，内存 {:.1}MB",
            stats.entries,
            stats.elapsed_ms / 1000.0,
            stats.bytes as f64 / 1048576.0
        );

        if let Some(path) = cache.as_deref() {
            let meta = persist::IndexMeta {
                built_at: now_secs(),
                roots: root_labels,
                entries: index.len(),
            };
            if let Err(err) = persist::save(&index, &meta, path) {
                eprintln!("[file] 索引落盘失败：{err}");
            }
        }

        if let Ok(mut guard) = self.index.write() {
            *guard = Some(index);
        }
        self.ready.store(true, Ordering::Relaxed);
    }

    pub fn search(&self, text: &str, limit: usize) -> Vec<Hit> {
        if !self.is_ready() || text.trim().is_empty() {
            return Vec::new();
        }
        let Ok(guard) = self.index.read() else {
            return Vec::new();
        };
        let Some(index) = guard.as_ref() else {
            return Vec::new();
        };
        let parsed = query::Query::parse(text);
        if parsed.is_empty() {
            return Vec::new();
        }
        query::search(index, &parsed, limit)
    }
}

fn default_roots() -> Vec<PathBuf> {
    if let Some(raw) = std::env::var_os("XTOOLS_INDEX_ROOTS") {
        let list: Vec<PathBuf> = std::env::split_paths(&raw).filter(|p| p.exists()).collect();
        if !list.is_empty() {
            return list;
        }
    }
    let profile = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Users\\Public"));
    vec![profile]
}

fn index_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|base| PathBuf::from(base).join("xtools").join("index").join("files.bin"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 在资源管理器里定位到该文件。
pub fn reveal(path: &str) -> Result<(), String> {
    let _ = path;
    std::process::Command::new("explorer.exe")
        .arg(format!("/select,{path}"))
        .spawn()
        .map(|_| ())
        .map_err(|err| err.to_string())
}

#[allow(dead_code)]
fn is_file(path: &Path) -> bool {
    path.is_file()
}
