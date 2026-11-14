//! NTFS 快速枚举：`FSCTL_ENUM_USN_DATA` 直接向卷要 MFT 全量记录。
//!
//! 相比逐目录遍历（2.4M 文件 ~36s），这条路径是内核顺序吐记录，通常 1~3s 完成，
//! 代价是需要管理员权限打开 `\\.\C:` 卷句柄。
//!
//! 记录里没有文件大小和修改时间（`FSCTL_ENUM_USN_DATA` 只给名字与父子关系），
//! 这在本工具里可以接受：列表只展示名字与路径，大小/时间不参与检索。

use std::collections::HashMap;

use crate::{Entry, Index, NO_PARENT};

const FSCTL_ENUM_USN_DATA: u32 = 0x0009_00B3;
const USN_RECORD_V2: u16 = 2;
const ROOT_FRN: u64 = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct RawRecord {
    pub frn: u64,
    pub parent: u64,
    pub name: String,
    pub is_dir: bool,
}

/// 解析 `FSCTL_ENUM_USN_DATA` 的输出缓冲。
///
/// 前 8 字节是下一次调用要用的起始 FRN，后面是连续的 `USN_RECORD_V2`。
pub fn parse_usn_buffer(buffer: &[u8], out: &mut Vec<RawRecord>) -> Option<u64> {
    if buffer.len() < 8 {
        return None;
    }
    let next = u64::from_le_bytes(buffer[..8].try_into().ok()?);

    let mut offset = 8usize;
    while offset + 8 <= buffer.len() {
        let length = u32::from_le_bytes(buffer[offset..offset + 4].try_into().ok()?) as usize;
        if length < 8 || offset + length > buffer.len() {
            break;
        }
        // 记录头（V2）：length 4 + major 2 + minor 2 + frn 8 + parent 8 + usn 8
        //                + reason 4 + source 4 + security 4 + attrs 4 + name_len 2 + name_off 2
        if length >= 60 {
            let major = u16::from_le_bytes(buffer[offset + 4..offset + 6].try_into().ok()?);
            if major == USN_RECORD_V2 {
                let frn = read_u64(buffer, offset + 8);
                let parent = read_u64(buffer, offset + 16);
                let attrs = u32::from_le_bytes(buffer[offset + 52..offset + 56].try_into().ok()?);
                let name_len = u16::from_le_bytes(buffer[offset + 56..offset + 58].try_into().ok()?)
                    as usize;
                let name_off = u16::from_le_bytes(buffer[offset + 58..offset + 60].try_into().ok()?)
                    as usize;

                if name_off + name_len <= length {
                    let bytes = &buffer[offset + name_off..offset + name_off + name_len];
                    let units: Vec<u16> = bytes
                        .chunks_exact(2)
                        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                        .collect();
                    let name = String::from_utf16_lossy(&units);
                    let is_dir = attrs & 0x10 != 0; // FILE_ATTRIBUTE_DIRECTORY
                    if !name.is_empty() {
                        out.push(RawRecord {
                            frn,
                            parent,
                            name,
                            is_dir,
                        });
                    }
                }
            }
        }
        offset += length;
    }

    Some(next)
}

fn read_u64(buffer: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(buffer[offset..offset + 8].try_into().unwrap_or([0; 8]))
}

/// 把扁平记录组装成「父下标 + 名字池」结构。
///
/// 父节点必须已经分配下标，因此分两趟：先登记所有 FRN，再回填父下标。
/// 同一个 FRN 可能有多个硬链接名，只保留第一个，避免重复行。
pub fn build_from_records(label: &str, records: Vec<RawRecord>) -> (Index, usize) {
    let mut index = Index::default();
    let mut slot: HashMap<u64, u32> = HashMap::with_capacity(records.len());

    let root_idx = index.push(NO_PARENT, label, true, 0, 0);
    slot.insert(ROOT_FRN, root_idx);

    // 第一趟：按顺序分配下标
    let mut pending: Vec<(u32, u64)> = Vec::with_capacity(records.len());
    let mut seen: HashMap<u64, u32> = HashMap::with_capacity(records.len());
    let mut duplicates = 0usize;

    for record in records {
        if record.frn == ROOT_FRN {
            continue;
        }
        if seen.contains_key(&record.frn) {
            duplicates += 1;
            continue;
        }
        let idx = index.push(NO_PARENT, &record.name, record.is_dir, 0, 0);
        seen.insert(record.frn, idx);
        pending.push((idx, record.parent));
    }

    // 第二趟：回填父下标（父节点可能出现在子节点之后）
    for (idx, parent_frn) in &pending {
        let parent = if *parent_frn == ROOT_FRN {
            root_idx
        } else {
            match seen.get(parent_frn) {
                Some(p) => *p,
                // 父节点不在枚举结果里（权限/孤立记录），挂到根
                None => root_idx,
            }
        };
        index.entries[*idx as usize].parent = parent;
    }

    (index, duplicates)
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::io;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::IO::DeviceIoControl;

    use super::{parse_usn_buffer, RawRecord, FSCTL_ENUM_USN_DATA};

    #[repr(C)]
    struct MftEnumData {
        start_file_reference_number: u64,
        low_usn: i64,
        high_usn: i64,
    }

    /// 枚举一个卷的 MFT 记录。调用者需要有管理员权限。
    pub fn enumerate_volume(letter: char) -> io::Result<Vec<RawRecord>> {
        let path: Vec<u16> = format!("\\\\.\\{letter}:")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|err| io::Error::from_raw_os_error(err.code().0))?;

        if handle.is_invalid() {
            return Err(io::Error::new(io::ErrorKind::Other, "无法打开卷句柄"));
        }

        let mut records = Vec::with_capacity(1 << 20);
        let mut input = MftEnumData {
            start_file_reference_number: 0,
            low_usn: 0,
            high_usn: i64::MAX,
        };

        let mut buffer = vec![0u8; 1 << 20];
        let mut guard = HandleGuard(handle);

        loop {
            let mut returned: u32 = 0;
            let ok = unsafe {
                DeviceIoControl(
                    guard.0,
                    FSCTL_ENUM_USN_DATA,
                    Some(&input as *const _ as *const c_void),
                    std::mem::size_of::<MftEnumData>() as u32,
                    Some(buffer.as_mut_ptr() as *mut c_void),
                    buffer.len() as u32,
                    Some(&mut returned),
                    None,
                )
            };

            if ok.is_err() || returned == 0 {
                // 枚举结束（ERROR_HANDLE_EOF）或出错，停止即可
                break;
            }

            let slice = &buffer[..returned as usize];
            let Some(next) = parse_usn_buffer(slice, &mut records) else {
                break;
            };
            if next <= input.start_file_reference_number {
                break;
            }
            input.start_file_reference_number = next;
        }

        Ok(records)
    }

    struct HandleGuard(HANDLE);

    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// 当前进程是否有管理员权限（读取卷句柄的前提）。
    pub fn is_elevated() -> bool {
        use windows::Win32::Security::{
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION::default();
            let mut size = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut c_void),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut size,
            );
            let _ = CloseHandle(token);
            ok.is_ok() && elevation.TokenIsElevated != 0
        }
    }
}

#[cfg(windows)]
pub use win::{enumerate_volume, is_elevated};

/// 列出可索引的固定磁盘卷。
pub fn fixed_volumes() -> Vec<char> {
    #[cfg(windows)]
    {
        use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
        const DRIVE_FIXED: u32 = 3;

        let mask = unsafe { GetLogicalDrives() };
        let mut volumes = Vec::new();
        for bit in 0..26 {
            if mask & (1 << bit) == 0 {
                continue;
            }
            let letter = (b'A' + bit as u8) as char;
            let path: Vec<u16> = format!("{letter}:\\")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let kind = unsafe { GetDriveTypeW(PCWSTR(path.as_ptr())) };
            if kind == DRIVE_FIXED {
                volumes.push(letter);
            }
        }
        volumes
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
use windows::core::PCWSTR;

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(frn: u64, parent: u64, name: &str, dir: bool) -> Vec<u8> {
        let utf16: Vec<u16> = name.encode_utf16().collect();
        let name_bytes = utf16.len() * 2;
        let length = (60 + name_bytes + 7) / 8 * 8;
        let mut buf = vec![0u8; length];

        buf[0..4].copy_from_slice(&(length as u32).to_le_bytes());
        buf[4..6].copy_from_slice(&USN_RECORD_V2.to_le_bytes());
        buf[8..16].copy_from_slice(&frn.to_le_bytes());
        buf[16..24].copy_from_slice(&parent.to_le_bytes());
        let attrs: u32 = if dir { 0x10 } else { 0x80 };
        buf[52..56].copy_from_slice(&attrs.to_le_bytes());
        buf[56..58].copy_from_slice(&(name_bytes as u16).to_le_bytes());
        buf[58..60].copy_from_slice(&60u16.to_le_bytes());
        for (i, unit) in utf16.iter().enumerate() {
            buf[60 + i * 2..62 + i * 2].copy_from_slice(&unit.to_le_bytes());
        }
        buf
    }

    #[test]
    fn parse_records() {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&99u64.to_le_bytes()); // next FRN
        buffer.extend_from_slice(&make_record(10, 5, "Users", true));
        buffer.extend_from_slice(&make_record(11, 10, "me", true));
        buffer.extend_from_slice(&make_record(12, 11, "a.txt", false));

        let mut out = Vec::new();
        let next = parse_usn_buffer(&buffer, &mut out).unwrap();
        assert_eq!(next, 99);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2].name, "a.txt");
        assert!(!out[2].is_dir);
        assert!(out[0].is_dir);
    }

    #[test]
    fn build_tree_with_out_of_order_parents() {
        let records = vec![
            RawRecord {
                frn: 12,
                parent: 11,
                name: "a.txt".into(),
                is_dir: false,
            },
            RawRecord {
                frn: 11,
                parent: 10,
                name: "me".into(),
                is_dir: true,
            },
            RawRecord {
                frn: 10,
                parent: 5,
                name: "Users".into(),
                is_dir: true,
            },
        ];
        let (index, dupes) = build_from_records("C:", records);
        assert_eq!(dupes, 0);
        assert_eq!(index.len(), 4);
        assert_eq!(index.path_of(1), "C:\\Users\\me\\a.txt");
        assert_eq!(index.path_of(3), "C:\\Users");
    }

    #[test]
    fn duplicate_frn_kept_once() {
        let records = vec![
            RawRecord {
                frn: 7,
                parent: 5,
                name: "one.txt".into(),
                is_dir: false,
            },
            RawRecord {
                frn: 7,
                parent: 5,
                name: "two.txt".into(),
                is_dir: false,
            },
        ];
        let (index, dupes) = build_from_records("C:", records);
        assert_eq!(dupes, 1);
        assert_eq!(index.len(), 2);
    }
}
