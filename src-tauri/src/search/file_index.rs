//! 文件索引状态与后台构建。
//!
//! 当前版本用目录遍历建索引（无需管理员），落盘后下次启动直接加载。
//! MFT/USN 的提权索引服务作为下一阶段接入，接口保持不变。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

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

    fn load_or_build(self: &Arc<Self>) {
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

        let index = self.build();

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

    fn build(self: &Arc<Self>) -> Index {
        // 有管理员权限时优先走 MFT 枚举（秒级），否则退回目录遍历
        if xtools_indexer::usn::is_elevated() {
            if let Some(volume) = xtools_indexer::profile_volume() {
                match xtools_indexer::usn::enumerate_volume(volume) {
                    Ok(records) => {
                        if !xtools_indexer::usn::looks_sane(&records) {
                            eprintln!(
                                "[file] MFT 枚举结果异常（{} 条），放弃并回退目录扫描",
                                records.len()
                            );
                            return self.build_by_walk();
                        }
                        let started = std::time::Instant::now();
                        let raw = records.len();
                        let (mft, duplicates) =
                            xtools_indexer::usn::build_from_records(&format!("{volume}:"), records);
                        let index = mft.index;
                        eprintln!(
                            "[file] MFT 索引 {} 条（原始 {raw}，去重 {duplicates}），用时 {:.1}s",
                            index.len(),
                            started.elapsed().as_secs_f64()
                        );

                        // 起监听线程：USN 日志增量刷新内存索引
                        match xtools_indexer::usn::journal::open(volume) {
                            Ok(info) => {
                                self.spawn_watcher(volume, info.id, info.next_usn, mft.frn_to_idx);
                            }
                            Err(err) => eprintln!("[file] 打开 USN 日志失败：{err}"),
                        }
                        return index;
                    }
                    Err(err) => eprintln!("[file] MFT 枚举失败（{err}），回退目录扫描"),
                }
            }
        }

        let (index, stats) = build_walk(&self.roots);
        eprintln!(
            "[file] 目录扫描索引 {} 条，用时 {:.1}s，内存 {:.1}MB",
            stats.entries,
            stats.elapsed_ms / 1000.0,
            stats.bytes as f64 / 1048576.0
        );
        index
    }

    fn build_by_walk(&self) -> Index {
        let (index, stats) = build_walk(&self.roots);
        eprintln!(
            "[file] 目录扫描索引 {} 条，用时 {:.1}s，内存 {:.1}MB",
            stats.entries,
            stats.elapsed_ms / 1000.0,
            stats.bytes as f64 / 1048576.0
        );
        index
    }

    /// 增量监听：每 500ms 读一次 USN 日志，把 create/rename/delete 应用到内存索引。
    fn spawn_watcher(
        self: &Arc<Self>,
        volume: char,
        journal_id: u64,
        start_usn: i64,
        frn_to_idx: HashMap<u64, u32>,
    ) {
        let me = Arc::clone(self);
        let state = Arc::new(Mutex::new((frn_to_idx, start_usn)));

        std::thread::Builder::new()
            .name("xtools-usn-watch".into())
            .spawn(move || {
                eprintln!("[file] USN 增量监听已启动（{volume}:）");
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    let since = state.lock().unwrap().1;
                    let (changes, next) =
                        match xtools_indexer::usn::journal::read(volume, journal_id, since) {
                            Ok(result) => result,
                            Err(err) => {
                                eprintln!("[file] 读取 USN 失败：{err}");
                                std::thread::sleep(std::time::Duration::from_secs(5));
                                continue;
                            }
                        };
                    if changes.is_empty() {
                        continue;
                    }

                    let mut guard = state.lock().unwrap();
                    let map = &mut guard.0;
                    if let Ok(mut index_guard) = me.index.write() {
                        if let Some(index) = index_guard.as_mut() {
                            let (added, removed) =
                                xtools_indexer::usn::apply_changes_split(index, map, &changes);
                            if added > 0 || removed > 0 {
                                if crate::debug_on() {
                                    eprintln!("[file] 增量 +{added} -{removed}");
                                }
                            }
                        }
                    }
                    guard.1 = next;
                }
            })
            .ok();
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

    // 设置里配置的根目录优先于默认的用户目录
    let configured: Vec<PathBuf> = crate::settings::current()
        .index_roots
        .iter()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .collect();
    if !configured.is_empty() {
        return configured;
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
