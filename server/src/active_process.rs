//! Best-effort foreground-child name for a shell PID.
//! Linux & macOS: `pgrep -n -P <pid>` then `ps -o comm= -p <child>`.
//! Returns "(idle)" on any failure or no children — same contract as the
//! TypeScript `activeProcess` helper.

use tokio::process::Command;

pub async fn active_process(shell_pid: u32) -> String {
    match active_process_inner(shell_pid).await {
        Some(s) if !s.is_empty() => s,
        _ => "(idle)".to_string(),
    }
}

async fn active_process_inner(shell_pid: u32) -> Option<String> {
    let pgrep = Command::new("pgrep")
        .arg("-n")
        .arg("-P")
        .arg(shell_pid.to_string())
        .output()
        .await
        .ok()?;
    if !pgrep.status.success() {
        return None;
    }
    let child = String::from_utf8_lossy(&pgrep.stdout).trim().to_string();
    if child.is_empty() {
        return None;
    }
    let ps = Command::new("ps")
        .arg("-o")
        .arg("comm=")
        .arg("-p")
        .arg(&child)
        .output()
        .await
        .ok()?;
    if !ps.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&ps.stdout).trim().to_string())
}
