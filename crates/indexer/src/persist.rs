//! 索引落盘：启动时直接加载，避免每次开机重扫几十秒。
//!
//! 格式刻意保持扁平，header + names 池 + 定长 entry 数组，加载即 memcpy。

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Entry, Index, NO_PARENT};

const MAGIC: &[u8; 8] = b"XTIDX01\0";
const VERSION: u32 = 1;
const ENTRY_BYTES: usize = 28;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexMeta {
    pub built_at: u64,
    pub roots: Vec<String>,
    pub entries: usize,
}

impl IndexMeta {
    pub fn age_secs(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now.saturating_sub(self.built_at)
    }
}

pub fn save(index: &Index, meta: &IndexMeta, path: &Path) -> io::Result<()> {
    let mut out = Vec::with_capacity(32 + index.names.len() + index.entries.len() * ENTRY_BYTES);

    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&[0u8; 4]); // 预留
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&meta.built_at.to_le_bytes());

    out.extend_from_slice(&(meta.roots.len() as u32).to_le_bytes());
    for root in &meta.roots {
        out.extend_from_slice(&(root.len() as u32).to_le_bytes());
        out.extend_from_slice(root.as_bytes());
    }

    out.extend_from_slice(&(index.entries.len() as u32).to_le_bytes());
    out.extend_from_slice(&(index.names.len() as u32).to_le_bytes());
    out.extend_from_slice(&index.names);

    for entry in &index.entries {
        out.extend_from_slice(&entry.parent.to_le_bytes());
        out.extend_from_slice(&entry.name_off.to_le_bytes());
        out.extend_from_slice(&entry.name_len.to_le_bytes());
        out.push(entry.is_dir as u8);
        out.push(0);
        out.extend_from_slice(&entry.size.to_le_bytes());
        out.extend_from_slice(&entry.mtime.to_le_bytes());
    }

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // 先写临时文件再改名，避免半截文件把下次启动搞坏
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &out)?;
    std::fs::rename(&tmp, path)
}

pub fn load(path: &Path) -> Option<(Index, IndexMeta)> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 24 || &data[0..8] != MAGIC {
        return None;
    }

    let mut cursor = 8usize;
    cursor += 4; // 预留

    let version = read_u32(&data, &mut cursor)?;
    if version != VERSION {
        return None;
    }

    let built_at = read_u64(&data, &mut cursor)?;
    let roots_len = read_u32(&data, &mut cursor)? as usize;
    let mut roots = Vec::with_capacity(roots_len);
    for _ in 0..roots_len {
        let len = read_u32(&data, &mut cursor)? as usize;
        let bytes = data.get(cursor..cursor + len)?;
        roots.push(String::from_utf8(bytes.to_vec()).ok()?);
        cursor += len;
    }

    let entries_len = read_u32(&data, &mut cursor)? as usize;
    let names_len = read_u32(&data, &mut cursor)? as usize;
    if data.len() < cursor + names_len + entries_len * ENTRY_BYTES {
        return None;
    }

    let names = data[cursor..cursor + names_len].to_vec();
    cursor += names_len;

    let mut entries = Vec::with_capacity(entries_len);
    for _ in 0..entries_len {
        let parent = read_u32(&data, &mut cursor)?;
        let name_off = read_u32(&data, &mut cursor)?;
        let name_len = read_u16(&data, &mut cursor)?;
        let is_dir = data.get(cursor).copied()? != 0;
        cursor += 2;
        let size = read_u64(&data, &mut cursor)?;
        let mtime = read_u64(&data, &mut cursor)?;

        if parent != NO_PARENT && parent as usize >= entries_len {
            return None;
        }
        if name_off as usize + name_len as usize > names_len {
            return None;
        }
        entries.push(Entry {
            parent,
            name_off,
            name_len,
            is_dir,
            size,
            mtime,
        });
    }

    Some((
        Index { names, entries },
        IndexMeta {
            built_at,
            roots,
            entries: entries_len,
        },
    ))
}

fn read_u32(data: &[u8], cursor: &mut usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(*cursor..*cursor + 4)?.try_into().ok()?;
    *cursor += 4;
    Some(u32::from_le_bytes(bytes))
}

fn read_u16(data: &[u8], cursor: &mut usize) -> Option<u16> {
    let bytes: [u8; 2] = data.get(*cursor..*cursor + 2)?.try_into().ok()?;
    *cursor += 2;
    Some(u16::from_le_bytes(bytes))
}

fn read_u64(data: &[u8], cursor: &mut usize) -> Option<u64> {
    let bytes: [u8; 8] = data.get(*cursor..*cursor + 8)?.try_into().ok()?;
    *cursor += 8;
    Some(u64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut index = Index::default();
        index.names.extend_from_slice(b"C:\\");
        index.names.extend_from_slice(b"foo.txt");
        index.entries.push(Entry {
            parent: NO_PARENT,
            name_off: 0,
            name_len: 3,
            is_dir: true,
            size: 0,
            mtime: 1,
        });
        index.entries.push(Entry {
            parent: 0,
            name_off: 3,
            name_len: 7,
            is_dir: false,
            size: 42,
            mtime: 2,
        });

        let meta = IndexMeta {
            built_at: 123,
            roots: vec!["C:\\".into()],
            entries: 2,
        };
        let dir = std::env::temp_dir().join("xtools-idx-test");
        let path = dir.join("i.bin");
        save(&index, &meta, &path).unwrap();

        let (loaded, loaded_meta) = load(&path).unwrap();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.name(1), "foo.txt");
        assert_eq!(loaded.path_of(1), "C:\\foo.txt");
        assert_eq!(loaded_meta.built_at, 123);
        assert_eq!(loaded.entries[1].size, 42);
    }
}
