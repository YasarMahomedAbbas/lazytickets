//! tmux integration for the start-work flow (M5). The only module that spawns
//! `tmux`: detect the current session, guard against a busy Claude pane, and
//! drive the project's `claude` window with send-keys.

use anyhow::{Context, Result};
use std::process::Output;
use tokio::process::Command;

async fn tmux(args: &[&str]) -> Result<Output> {
    Command::new("tmux")
        .args(args)
        .output()
        .await
        .context("failed to spawn `tmux` — is it installed and are we inside a server?")
}

/// The current tmux session name, or `None` if we're not running inside tmux.
pub async fn current_session() -> Result<Option<String>> {
    if std::env::var_os("TMUX").is_none() {
        return Ok(None);
    }
    let out = tmux(&["display-message", "-p", "#S"]).await?;
    if !out.status.success() {
        return Ok(None);
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((!name.is_empty()).then_some(name))
}

/// Whether `session` has a window named `claude` to drive.
pub async fn has_claude_window(session: &str) -> Result<bool> {
    let out = tmux(&["list-windows", "-t", session, "-F", "#{window_name}"]).await?;
    if !out.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().any(|w| w == "claude"))
}

/// Whether Claude is busy in `session`, using the same pane-ownership check as
/// `sesh-list-bells`: `@claude_busy` holds the pane id that set it, and it only
/// counts if that pane still exists and lives in this session (else it's stale).
pub async fn is_busy(session: &str) -> Result<bool> {
    let out = tmux(&["show-options", "-t", session, "-qv", "@claude_busy"]).await?;
    let pane = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if pane.is_empty() {
        return Ok(false);
    }
    let owner = tmux(&["display-message", "-p", "-t", &pane, "#{session_name}"]).await?;
    if !owner.status.success() {
        return Ok(false); // pane gone → stale flag
    }
    Ok(String::from_utf8_lossy(&owner.stdout).trim() == session)
}

/// Clear the claude pane and invoke `skill` for `issue`. Two lines: `/clear`
/// then an explicit skill instruction — deterministic, no trigger-guessing.
pub async fn start_work(session: &str, skill: &str, issue: u64) -> Result<()> {
    let target = format!("{session}:claude");
    send_line(&target, "/clear").await?;
    send_line(&target, &format!("Use the {skill} skill for issue #{issue}")).await?;
    Ok(())
}

/// Send `line` literally (`-l`, no key-name interpretation) followed by Enter.
async fn send_line(target: &str, line: &str) -> Result<()> {
    run_ok(&["send-keys", "-t", target, "-l", line]).await?;
    run_ok(&["send-keys", "-t", target, "Enter"]).await?;
    Ok(())
}

async fn run_ok(args: &[&str]) -> Result<()> {
    let out = tmux(args).await?;
    if !out.status.success() {
        anyhow::bail!("`tmux {}` failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}
