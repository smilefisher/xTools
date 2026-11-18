//! 应用索引：开始菜单快捷方式 + App Paths 注册表 → 内存索引 → 模糊搜索。
//!
//! 设计取舍：
//! - 不用 IShellLink 解析 .lnk，直接拿 .lnk 路径交给 ShellExecute，
//!   这样工作目录、参数、图标全部由系统负责，省掉一大坨 COM 与转义逻辑。
//! - 索引常驻内存，搜索不碰磁盘；图标按需提取并缓存成 PNG。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use pinyin::ToPinyin;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
pub struct AppEntry {
    pub id: u32,
    pub name: String,
    /// 交给 ShellExecute 的路径（.lnk 或 .exe）
    pub path: String,
    pub source: &'static str,
    #[serde(skip)]
    key_name: String,
    #[serde(skip)]
    key_initials: String,
    #[serde(skip)]
    key_pinyin: String,
    /// 别名键：快捷方式指向的 exe 名（如 微信 → wechat），用于英文名搜索
    key_alias: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AppHit {
    pub id: u32,
    pub name: String,
    pub target: String,
    pub score: u32,
}

#[derive(Default, Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Usage {
    pub count: u32,
    pub last_used: u64,
}

pub struct AppIndex {
    pub entries: Vec<AppEntry>,
    pub built_ms: f64,
}

impl AppIndex {
    pub fn build() -> Self {
        let started = std::time::Instant::now();
        let mut raw: Vec<(String, String, &'static str)> = Vec::new();

        for lnk in start_menu_shortcuts() {
            let Some(stem) = lnk.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if should_skip(stem) {
                continue;
            }
            raw.push((stem.to_string(), lnk.to_string_lossy().into_owned(), "startmenu"));
        }

        for (name, exe) in app_paths() {
            raw.push((name, exe, "apppath"));
        }

        // UWP 应用：AUMID 交给 shell:AppsFolder 启动
        for (name, aumid) in uwp_apps() {
            raw.push((name, format!("shell:AppsFolder\\{aumid}"), "uwp"));
        }

        // 同名只留一个（开始菜单优先，因为 .lnk 自带工作目录与参数）
        raw.sort_by_key(|(_, _, source)| if *source == "startmenu" { 0 } else { 1 });
        let mut seen: HashSet<String> = HashSet::new();
        let mut entries = Vec::with_capacity(raw.len());

        for (name, path, source) in raw {
            let dedupe = name.to_lowercase();
            if !seen.insert(dedupe) {
                continue;
            }
            let (key_name, key_initials, key_pinyin) = make_keys(&name);
            let key_alias = if source == "startmenu" {
                let lowered = name.to_lowercase();
                lnk_exe_stems(&path)
                    .into_iter()
                    .filter(|stem| !lowered.contains(stem.as_str()))
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                String::new()
            };
            entries.push(AppEntry {
                id: fnv1a(path.as_bytes()),
                name,
                path,
                source,
                key_name,
                key_initials,
                key_pinyin,
                key_alias,
            });
        }

        Self {
            entries,
            built_ms: started.elapsed().as_secs_f64() * 1000.0,
        }
    }

    pub fn search(&self, query: &str, limit: usize, usage: &HashMap<u32, Usage>) -> Vec<AppHit> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.top_by_usage(limit, usage);
        }

        let mut hits: Vec<AppHit> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let base = match_score(entry, &q)?;
                let usage_score = usage
                    .get(&entry.id)
                    .map(|u| (u.count.min(50) * 4) as u32 + recency_bonus(u.last_used))
                    .unwrap_or(0);
                Some(AppHit {
                    id: entry.id,
                    name: entry.name.clone(),
                    target: entry.path.clone(),
                    score: base + usage_score,
                })
            })
            .collect();

        hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.name.cmp(&b.name)));
        hits.truncate(limit);
        hits
    }

    fn top_by_usage(&self, limit: usize, usage: &HashMap<u32, Usage>) -> Vec<AppHit> {
        let mut ranked: Vec<(&AppEntry, i64)> = self
            .entries
            .iter()
            .map(|entry| {
                let score = usage
                    .get(&entry.id)
                    .map(|u| u.count as i64 * 10 + recency_bonus(u.last_used) as i64)
                    .unwrap_or(i64::MIN / 2);
                (entry, score)
            })
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.name.cmp(&b.0.name)));

        ranked
            .into_iter()
            .take(limit)
            .map(|(entry, _)| AppHit {
                id: entry.id,
                name: entry.name.clone(),
                target: entry.path.clone(),
                score: 0,
            })
            .collect()
    }

    pub fn find(&self, id: u32) -> Option<&AppEntry> {
        self.entries.iter().find(|e| e.id == id)
    }
}

fn recency_bonus(last_used: u64) -> u32 {
    if last_used == 0 {
        return 0;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let age = now.saturating_sub(last_used);
    match age {
        0..=3_600 => 30,
        3_601..=86_400 => 20,
        86_401..=604_800 => 10,
        _ => 0,
    }
}

/// 从 .lnk 文件里提取目标 exe 的名字（不含扩展名），用于英文名搜索。
///
/// 不调用 COM：快捷方式的目标路径本来就以 ANSI/UTF-16 明文存在文件里，
/// 直接扫 `*.exe` 更省事，也不受 Shell API 签名变化影响。
fn lnk_exe_stems(path: &str) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let mut candidates: Vec<String> = Vec::new();

    // ANSI：向前找到路径起点，取最后一段
    let lower: Vec<u8> = bytes.iter().map(|b| b.to_ascii_lowercase()).collect();
    let mut i = 0usize;
    while i + 4 <= lower.len() {
        if &lower[i..i + 4] == b".exe" {
            let mut start = i;
            while start > 0 {
                let c = lower[start - 1];
                let ok = c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.' || c == b'\\' || c == b'/';
                if !ok {
                    break;
                }
                start -= 1;
            }
            if let Ok(text) = std::str::from_utf8(&bytes[start..i + 4]) {
                if let Some(stem) = text.rsplit(['\\', '/']).next() {
                    let stem = stem.trim_end_matches(".exe").trim_end_matches(".EXE");
                    if !stem.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
                        candidates.push(stem.to_ascii_lowercase());
                    }
                }
            }
            i += 4;
        } else {
            i += 1;
        }
    }

    // UTF-16LE 形式（较新的 shell 会写这份）
    let mut j = 0usize;
    while j + 8 <= bytes.len() {
        if bytes[j] == b'.' && bytes[j + 2] == b'e' && bytes[j + 4] == b'x' && bytes[j + 6] == b'e'
            && bytes[j + 1] == 0 && bytes[j + 3] == 0 && bytes[j + 5] == 0 && bytes[j + 7] == 0
        {
            let mut start = j;
            while start >= 2 && bytes[start - 1] == 0 && bytes[start - 2].is_ascii_alphanumeric() {
                start -= 2;
            }
            let units: Vec<u16> = bytes[start..j]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            if let Ok(text) = String::from_utf16(&units) {
                let stem = text.trim_end_matches(".exe").to_ascii_lowercase();
                if !stem.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
                    candidates.push(stem);
                }
            }
            j += 8;
        } else {
            j += 1;
        }
    }

    candidates.retain(|stem| stem.len() >= 3 && stem != "exe");
    candidates.sort();
    candidates.dedup();
    candidates
}

fn make_keys(name: &str) -> (String, String, String) {
    let lower = name.to_lowercase();
    let mut initials = String::with_capacity(name.chars().count());
    let mut full = String::with_capacity(name.chars().count() * 2);
    for ch in name.chars() {
        match ch.to_pinyin() {
            Some(py) => {
                initials.push_str(py.first_letter());
                full.push_str(py.plain());
            }
            None => {
                if ch.is_alphanumeric() {
                    initials.push(ch.to_ascii_lowercase());
                    full.push(ch.to_ascii_lowercase());
                }
            }
        }
    }
    (lower, initials.to_lowercase(), full.to_lowercase())
}

/// 打分：完全匹配 > 前缀 > 词界 > 子串 > 拼音首字母 > 全拼 > 子序列
fn match_score(entry: &AppEntry, q: &str) -> Option<u32> {
    let name = &entry.key_name;

    if name == q {
        return Some(1_000);
    }
    if name.starts_with(q) {
        return Some(900 - (name.chars().count() - q.chars().count()).min(60) as u32);
    }
    if let Some(pos) = name.find(q) {
        let boundary = pos > 0
            && name[..pos]
                .chars()
                .next_back()
                .map(|c| c == ' ' || c == '-' || c == '_' || c == '.')
                .unwrap_or(false);
        return Some(if boundary { 850 } else { 800 - pos.min(50) as u32 });
    }
    if entry.key_initials == q {
        return Some(790);
    }
    if entry.key_initials.starts_with(q) {
        return Some(760);
    }
    if entry.key_initials.contains(q) {
        return Some(700);
    }
    if entry.key_pinyin.contains(q) {
        return Some(640);
    }
    // 别名：目标 exe 名（微信 → wechat）
    if !entry.key_alias.is_empty() {
        if entry.key_alias == q {
            return Some(820);
        }
        if entry.key_alias.starts_with(q) {
            return Some(720);
        }
        if entry.key_alias.contains(q) {
            return Some(600);
        }
    }
    // 子序列：typo 容忍（如 vs → visual studio code）
    subsequence_score(name, q)
}

fn subsequence_score(haystack: &str, needle: &str) -> Option<u32> {
    let hay: Vec<char> = haystack.chars().collect();
    let mut idx = 0usize;
    let mut gaps = 0usize;
    for want in needle.chars() {
        let mut found = None;
        for i in idx..hay.len() {
            if hay[i] == want {
                found = Some(i);
                break;
            }
        }
        let pos = found?;
        gaps += pos - idx;
        idx = pos + 1;
    }
    if gaps > needle.chars().count() * 3 {
        return None;
    }
    Some(420u32.saturating_sub(gaps.min(120) as u32))
}

fn should_skip(name: &str) -> bool {
    const SKIP: [&str; 10] = [
        "uninstall",
        "卸载",
        "readme",
        "说明",
        "help",
        "帮助",
        "website",
        "网站",
        "release notes",
        "更新日志",
    ];
    let lower = name.to_lowercase();
    SKIP.iter().any(|s| lower.contains(s))
}

fn start_menu_shortcuts() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(program_data) = std::env::var_os("ProgramData") {
        roots.push(
            PathBuf::from(program_data)
                .join("Microsoft\\Windows\\Start Menu\\Programs"),
        );
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs"));
    }

    let mut found = Vec::new();
    for root in roots {
        walk(&root, &mut found, 0);
    }
    found
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out, depth + 1);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("lnk"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

/// App Paths：注册表里登记的可执行文件（很多开发工具只有这里，没有开始菜单项）
fn app_paths() -> Vec<(String, String)> {
    const ROOT: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
    const WOW: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths";

    let mut result = Vec::new();
    for hive in [winreg::enums::HKEY_CURRENT_USER, winreg::enums::HKEY_LOCAL_MACHINE] {
        for root in [ROOT, WOW] {
            let Ok(key) = winreg::RegKey::predef(hive).open_subkey(root) else {
                continue;
            };
            for sub in key.enum_keys().flatten() {
                let Ok(item) = key.open_subkey(&sub) else {
                    continue;
                };
                let Ok(exe) = item.get_value::<String, _>("") else {
                    continue;
                };
                if !exe.to_lowercase().ends_with(".exe") || !Path::new(&exe).exists() {
                    continue;
                }
                let name = Path::new(&exe)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&sub)
                    .to_string();
                result.push((name, exe));
            }
        }
    }
    result
}

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in bytes {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

// ---------------------------------------------------------------- 启动

/// 当前进程是否有管理员权限（安装计划任务时用）。
pub fn is_elevated_process() -> bool {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::Security::{
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        unsafe {
            let mut token = windows::Win32::Foundation::HANDLE::default();
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
    #[cfg(not(windows))]
    {
        false
    }
}

/// 枚举 UWP/Store 应用：开始菜单里没有它们，但用户搜「计算器」「设置」时必须有。
///
/// 走 `Get-StartApps`（Shell 的 AppsFolder 视图）而不是 WinRT `PackageManager`：
/// 后者在非交互/锁屏状态下会返回 ACCESS_DENIED（实测），前者稳定且自带显示名。
/// 结果缓存到磁盘，启动时直接读，后台再异步刷新。
pub fn uwp_apps() -> Vec<(String, String)> {
    if let Some(cached) = load_uwp_cache() {
        return cached;
    }

    // 缓存缺失时不能在启动路径上同步跑 PowerShell（实测会卡 3.9 秒），
    // 改为后台抓取并落盘，下次启动生效
    std::thread::spawn(|| {
        let started = std::time::Instant::now();
        let fresh = fetch_uwp_apps();
        let count = fresh.len();
        save_uwp_cache(&fresh);
        eprintln!(
            "[uwp] 后台抓取 {count} 个 UWP 应用，用时 {:.1}s",
            started.elapsed().as_secs_f64()
        );
    });

    Vec::new()
}

fn fetch_uwp_apps() -> Vec<(String, String)> {
    // 通过临时文件取结果：PowerShell 的 stdout 走控制台代码页（GBK），
    // 直接管道读回来中文会变乱码；文件用 UTF-8 写就没有这个问题。
    let temp = std::env::temp_dir().join("xtools-uwp.json");
    let script = format!(
        "Get-StartApps | Select-Object Name,AppID | ConvertTo-Json -Compress | Set-Content -Path '{}' -Encoding UTF8",
        temp.display()
    );

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let raw = match std::fs::read(&temp) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };
    let _ = std::fs::remove_file(&temp);

    // 去掉 UTF-8 BOM
    let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&raw);
    let text = String::from_utf8_lossy(raw);
    parse_start_apps(&text)
}

fn parse_start_apps(text: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return Vec::new();
    };

    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(_) => vec![value],
        _ => return Vec::new(),
    };

    let mut result = Vec::with_capacity(items.len());
    for item in items {
        let name = item.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let app_id = item.get("AppID").and_then(|v| v.as_str()).unwrap_or("").to_string();
        // UWP 的 AUMID 形如 `PackageFamilyName!AppId`；桌面程序的 AppID 是
        // "Chrome" 这类别名或可执行文件名，已经在开始菜单索引里了，这里跳过
        if name.is_empty() || !app_id.contains('!') {
            continue;
        }
        result.push((name, app_id));
    }
    result
}

fn uwp_cache_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|base| PathBuf::from(base).join("xtools").join("index").join("uwp.json"))
}

fn load_uwp_cache() -> Option<Vec<(String, String)>> {
    let path = uwp_cache_path()?;
    let meta = std::fs::metadata(&path).ok()?;
    let age = meta
        .modified()
        .ok()
        .and_then(|time| time.elapsed().ok())
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX);
    if age > 24 * 3600 {
        return None;
    }
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Vec<(String, String)>>(&raw).ok()
}

fn save_uwp_cache(apps: &[(String, String)]) {
    let Some(path) = uwp_cache_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(raw) = serde_json::to_string(apps) {
        let _ = std::fs::write(path, raw);
    }
}

/// 启动应用。放到后台线程执行：`ShellExecuteEx` 在命令线程上可能阻塞，
/// 而且即使失败也不该让面板卡住。
pub fn launch(path: &str, run_as_admin: bool) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("启动路径为空".into());
    }
    let target = path.to_string();
    std::thread::spawn(move || {
        if let Err(err) = launch_blocking(&target, run_as_admin) {
            eprintln!("[launch] {target} 启动失败：{err}");
        }
    });
    Ok(())
}

/// 同步版本，供命令行探针确认 ShellExecute 是否真的能拉起程序。
pub fn launch_blocking_for_test(path: &str) -> Result<(), String> {
    launch_blocking(path, false)
}

fn launch_blocking(path: &str, run_as_admin: bool) -> Result<(), String> {
    // Shell 系列 API 需要线程初始化 COM；初始化失败（已是其它模式）不影响后续调用
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }

    // UWP 用 shell: 协议交给资源管理器激活
    if path.starts_with("shell:") {
        return std::process::Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string());
    }

    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let file: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let verb: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();

        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            lpVerb: if run_as_admin {
                PCWSTR(verb.as_ptr())
            } else {
                PCWSTR::null()
            },
            lpFile: PCWSTR(file.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };

        unsafe { ShellExecuteExW(&mut info).map_err(|err| err.message().to_string()) }
    }
    #[cfg(not(windows))]
    {
        let _ = (path, run_as_admin);
        Err("仅支持 Windows".into())
    }
}

// ---------------------------------------------------------------- 图标

/// 提取 32px 图标并缓存成 PNG；返回 PNG 字节。
pub fn icon_png(path: &str, cache_dir: &Path) -> Option<Vec<u8>> {
    let id = fnv1a(path.as_bytes());
    let cache_file = cache_dir.join(format!("{id:08x}.png"));
    if let Ok(bytes) = std::fs::read(&cache_file) {
        return Some(bytes);
    }
    // UWP 的 `shell:AppsFolder\...` 走 shell 命名空间取图，SHGetFileInfo 不认这种路径
    let bytes = if path.starts_with("shell:") {
        shell_item_png(path)
    } else {
        extract_icon_png(path)
    }?;
    let _ = std::fs::create_dir_all(cache_dir);
    let _ = std::fs::write(&cache_file, &bytes);
    Some(bytes)
}

/// 用 `IShellItemImageFactory` 取 shell 项的 32px 图标（UWP 应用靠这个）。
#[cfg(windows)]
fn shell_item_png(path: &str) -> Option<Vec<u8>> {
    use std::ffi::c_void;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS, HBITMAP,
    };
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromIDList, SHParseDisplayName, SIIGBF_ICONONLY,
    };

    const SIZE_PX: i32 = 32;
    let debug = std::env::var_os("XTOOLS_DEBUG").is_some();
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut pidl = std::ptr::null_mut();

    unsafe {
        // shell 命名空间解析要求线程已初始化 COM
        let hr = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
        if debug {
            eprintln!("[icon] CoInitializeEx -> {hr:?}");
        }
        match SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None) {
            Ok(()) => {}
            Err(err) => {
                if debug {
                    eprintln!("[icon] SHParseDisplayName({path}) 失败：{err}");
                }
                return None;
            }
        }
    }
    if pidl.is_null() {
        if debug {
            eprintln!("[icon] pidl 为空：{path}");
        }
        return None;
    }

    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromIDList(pidl).ok()?;
        let hbmp: HBITMAP = factory
            .GetImage(
                SIZE {
                    cx: SIZE_PX,
                    cy: SIZE_PX,
                },
                SIIGBF_ICONONLY,
            )
            .ok()?;

        let screen = GetDC(None);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE_PX,
                biHeight: -SIZE_PX,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut pixels = vec![0u8; (SIZE_PX * SIZE_PX * 4) as usize];
        let lines = GetDIBits(
            screen,
            hbmp,
            0,
            SIZE_PX as u32,
            Some(pixels.as_mut_ptr() as *mut c_void),
            &mut info,
            DIB_RGB_COLORS,
        );

        let _ = DeleteObject(hbmp.into());
        ReleaseDC(None, screen);

        if lines == 0 {
            return None;
        }

        // BGRA -> RGBA
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        encode_png(&pixels, SIZE_PX as u32, SIZE_PX as u32)
    }
}

#[cfg(not(windows))]
fn shell_item_png(_path: &str) -> Option<Vec<u8>> {
    None
}

#[cfg(windows)]
fn extract_icon_png(path: &str) -> Option<Vec<u8>> {
    use std::ffi::c_void;
    use std::mem::size_of;

    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL};

    const SIZE: i32 = 32;

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut info = SHFILEINFOW::default();

    let ok = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            Default::default(),
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if ok == 0 || info.hIcon.is_invalid() {
        return None;
    }

    let icon = info.hIcon;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];

    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE,
                biHeight: -SIZE, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        if let Ok(bitmap) = CreateDIBSection(Some(mem), &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0)
        {
            let old = SelectObject(mem, bitmap.into());
            let _ = DrawIconEx(mem, 0, 0, icon, SIZE, SIZE, 0, None, DI_NORMAL);
            if !bits.is_null() {
                std::ptr::copy_nonoverlapping(bits as *const u8, rgba.as_mut_ptr(), rgba.len());
            }
            SelectObject(mem, old);
            let _ = DeleteObject(bitmap.into());
        }

        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        let _ = DestroyIcon(icon);
    }

    // BGRA -> RGBA
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }

    encode_png(&rgba, SIZE as u32, SIZE as u32)
}

#[cfg(windows)]
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    Some(out.into_inner())
}

// ---------------------------------------------------------------- 使用频次

#[derive(Default, Serialize, Deserialize)]
pub struct UsageStore {
    pub apps: HashMap<u32, Usage>,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

impl UsageStore {
    pub fn load() -> Self {
        let path = usage_path();
        let apps = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str::<HashMap<u32, Usage>>(&raw).ok())
            .unwrap_or_default();
        Self { apps, path }
    }

    pub fn record(&mut self, id: u32) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = self.apps.entry(id).or_default();
        entry.count += 1;
        entry.last_used = now;
        self.flush();
    }

    fn flush(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(raw) = serde_json::to_string(&self.apps) {
            let _ = std::fs::write(path, raw);
        }
    }
}

fn usage_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|base| base.join("xtools").join("usage.json"))
}

pub fn icon_cache_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("xtools").join("icons"))
}
