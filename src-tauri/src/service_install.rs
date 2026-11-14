//! 索引服务的安装/卸载：用计划任务（`schtasks`）拉起 `xtools-indexd.exe`。
//!
//! 为什么不是 Windows 服务：服务需要 SCM 那套封装与安装器，而计划任务
//! 「登录时启动 + 最高权限」已经能满足需求，还能让用户随时在任务计划程序里看到/禁用。

use std::path::PathBuf;
use std::process::Command;

pub const TASK_NAME: &str = "xToolsIndexService";

fn daemon_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join("xtools-indexd.exe");
    if candidate.exists() {
        return Some(candidate);
    }
    // 开发态：从 target/release 或 target/debug 找
    let dev = dir.join("xtools-indexd.exe");
    dev.exists().then_some(dev)
}

pub fn task_exists() -> bool {
    let output = Command::new("schtasks")
        .args(["/query", "/tn", TASK_NAME])
        .output();
    matches!(output, Ok(out) if out.status.success())
}

pub fn current_run_level() -> Option<String> {
    let output = Command::new("schtasks")
        .args(["/query", "/tn", TASK_NAME, "/xml"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let xml = String::from_utf8_lossy(&output.stdout);
    if xml.contains("HighestAvailable") {
        Some("HighestAvailable".into())
    } else {
        Some("Limited".into())
    }
}

/// 安装（或覆盖）计划任务并立刻启动。`elevated` 决定是否请求最高权限。
pub fn install(elevated: bool) -> Result<String, String> {
    let daemon = daemon_path().ok_or("找不到 xtools-indexd.exe（应和主程序同目录）")?;
    let tr = format!("\"{}\"", daemon.display());

    let mut command = Command::new("schtasks");
    command.args([
        "/create",
        "/tn",
        TASK_NAME,
        "/tr",
        &tr,
        "/sc",
        "ONLOGON",
        "/f",
    ]);
    if elevated {
        command.args(["/rl", "HIGHEST"]);
    }

    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("创建计划任务失败：{stdout}{stderr}").trim().to_string());
    }

    // 立刻启动，不必等下次登录
    let start = Command::new("schtasks")
        .args(["/run", "/tn", TASK_NAME])
        .output()
        .map_err(|e| e.to_string())?;
    let started = start.status.success();

    Ok(format!(
        "已安装计划任务 {}（{}），{}",
        TASK_NAME,
        if elevated { "最高权限" } else { "当前用户权限" },
        if started { "并已启动" } else { "但启动失败，可稍后手动运行" }
    ))
}

pub fn uninstall() -> Result<String, String> {
    if !task_exists() {
        let _ = Command::new("taskkill")
            .args(["/im", "xtools-indexd.exe", "/f"])
            .output();
        return Ok("计划任务本来就不存在，已确保助手进程结束".to_string());
    }

    let output = Command::new("schtasks")
        .args(["/delete", "/tn", TASK_NAME, "/f"])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("删除计划任务失败：{stdout}{stderr}").trim().to_string());
    }

    // 顺手结束还在跑的助手进程
    let _ = Command::new("taskkill")
        .args(["/im", "xtools-indexd.exe", "/f"])
        .output();

    Ok(format!("已删除计划任务 {TASK_NAME}，并结束助手进程"))
}
