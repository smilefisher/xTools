//! xTools 文件索引。
//!
//! 内存布局（对标 Everything 的取舍）：
//! - 不存完整路径，只存「父节点下标 + 名字在 names 池里的偏移」，展示时回溯拼装；
//! - 名字池是一整块 `Vec<u8>`，避免每条一个 `String` 的分配器开销；
//! - 查询走 rayon 并行扫描 + 早退，百万级名字 < 5ms。
//!
//! 两个数据来源：
//! - `walk`：普通目录遍历，无需管理员，作为默认与降级路径；
//! - `mft`：直接读 NTFS `$MFT`，需要管理员，百万文件 1~3s 建完（后续里程碑接入）。

pub mod persist;
pub mod query;
pub mod usn;

use std::path::{Path, PathBuf};

use serde::Serialize;

pub const NO_PARENT: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub parent: u32,
    pub name_off: u32,
    pub name_len: u16,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Default, Serialize)]
pub struct BuildStats {
    pub entries: usize,
    pub bytes: usize,
    pub elapsed_ms: f64,
    pub skipped_dirs: usize,
    pub errors: usize,
}

#[derive(Default)]
pub struct Index {
    pub names: Vec<u8>,
    pub entries: Vec<Entry>,
}

impl Index {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn name(&self, idx: u32) -> &str {
        let entry = &self.entries[idx as usize];
        let start = entry.name_off as usize;
        // SAFETY: 名字写入时保证是合法 UTF-8 片段
        std::str::from_utf8(&self.names[start..start + entry.name_len as usize]).unwrap_or("")
    }

    /// 回溯父节点拼出完整路径。深度很浅（一般 < 10），成本可忽略。
    pub fn path_of(&self, idx: u32) -> String {
        let mut parts: Vec<&str> = Vec::with_capacity(8);
        let mut cur = idx;
        loop {
            parts.push(self.name(cur));
            let parent = self.entries[cur as usize].parent;
            if parent == NO_PARENT {
                break;
            }
            cur = parent;
        }

        let mut out = String::with_capacity(parts.iter().map(|p| p.len() + 1).sum());
        for part in parts.iter().rev() {
            if !out.is_empty() && !out.ends_with('\\') && !part.starts_with('\\') {
                out.push('\\');
            }
            out.push_str(part);
        }
        out
    }

    pub fn memory_bytes(&self) -> usize {
        self.names.capacity() + self.entries.capacity() * std::mem::size_of::<Entry>()
    }

    /// 把另一个索引并入当前索引，自动修正父下标与名字偏移。
    pub fn append(&mut self, other: Index) {
        let entry_base = self.entries.len() as u32;
        let name_base = self.names.len() as u32;
        for entry in other.entries {
            let parent = if entry.parent == NO_PARENT {
                NO_PARENT
            } else {
                entry.parent + entry_base
            };
            self.entries.push(Entry {
                parent,
                name_off: entry.name_off + name_base,
                ..entry
            });
        }
        self.names.extend_from_slice(&other.names);
    }

    fn push(&mut self, parent: u32, name: &str, is_dir: bool, size: u64, mtime: u64) -> u32 {
        let offset = self.names.len() as u32;
        self.names.extend_from_slice(name.as_bytes());
        let idx = self.entries.len() as u32;
        self.entries.push(Entry {
            parent,
            name_off: offset,
            name_len: name.len() as u16,
            is_dir,
            size,
            mtime,
        });
        idx
    }
}

/// 用 MFT 枚举快速建索引（需要管理员）。任一环节失败都返回错误，由调用方降级。
pub fn build_usn(volumes: &[char]) -> std::io::Result<(Index, BuildStats)> {
    let started = std::time::Instant::now();
    let mut stats = BuildStats::default();
    let mut index = Index::default();

    for &volume in volumes {
        let records = usn::enumerate_volume(volume)?;
        let raw_count = records.len();
        let (part, duplicates) = usn::build_from_records(&format!("{volume}:"), records);
        eprintln!(
            "[mft] {volume}: 记录 {raw_count} 条，去重 {duplicates} 条，入索引 {} 条",
            part.len()
        );
        stats.bytes += part.memory_bytes();
        index.append(part);
    }

    stats.entries = index.len();
    stats.bytes = index.memory_bytes();
    stats.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((index, stats))
}

/// `%USERPROFILE%` 所在的卷盘符。
pub fn profile_volume() -> Option<char> {
    let profile = std::env::var("USERPROFILE").ok()?;
    profile.chars().next().filter(|c| c.is_ascii_alphabetic())
}

/// 目录遍历建索引（无需管理员）。
///
/// `roots` 里每个元素都会作为一个根节点加入，之后路径形如 `C:\Users\me\a.txt`。
pub fn build_walk(roots: &[PathBuf]) -> (Index, BuildStats) {
    let started = std::time::Instant::now();
    let mut index = Index {
        names: Vec::with_capacity(8 << 20),
        entries: Vec::with_capacity(200_000),
    };
    let mut stats = BuildStats::default();

    for root in roots {
        let label = root.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
        let root_idx = index.push(NO_PARENT, &label, true, 0, 0);
        walk(&mut index, root, root_idx, &mut stats);
    }

    stats.entries = index.len();
    stats.bytes = index.memory_bytes();
    stats.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    (index, stats)
}

fn walk(index: &mut Index, dir: &Path, parent_idx: u32, stats: &mut BuildStats) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        stats.skipped_dirs += 1;
        return;
    };

    for entry in read_dir {
        let Ok(entry) = entry else {
            stats.errors += 1;
            continue;
        };
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            stats.errors += 1;
            continue;
        };

        // 跳过重解析点（junction / symlink），否则容易绕成环
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                continue;
            }
        }

        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let is_dir = meta.is_dir();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let size = if is_dir { 0 } else { meta.len() };

        let idx = index.push(parent_idx, name, is_dir, size, mtime);
        if is_dir {
            walk(index, &path, idx, stats);
        }
    }
}
