//! tmux integration for the start-work flow (M5). The only module that spawns
//! `tmux`: detect the current session, guard against a busy Claude pane, and
//! drive the project's `claude` window with send-keys.

use anyhow::{Context, Result};
use std::path::Path;
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
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|w| w == "claude"))
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
    send_line(
        &target,
        &format!("Use the {skill} skill for issue #{issue}"),
    )
    .await?;
    Ok(())
}

/// Create a detached session named `session` rooted at `dir`, run any `setup`
/// commands, then launch Claude in its shell, seeded with the skill instruction
/// for `issue`.
///
/// The prompt is passed as an argument to `claude` rather than typed in with
/// send-keys after startup: the fresh shell is ready to accept the command
/// immediately, so we never race Claude's TUI initialisation, and a brand-new
/// session has nothing to `/clear`. Detached (`-d`) so lazytickets keeps focus —
/// you stay put and fire off the next ticket.
///
/// `setup` runs first under `sh -c` so any `cd` inside it can't move where
/// Claude launches, and Claude starts regardless of setup's exit status (`;`
/// not `&&`) — its scrollback shows a failed `npm install` rather than hiding
/// it. `sh -c` rather than a `( … )` subshell because the session runs the
/// user's login shell, which may be fish: fish parses `( … )` in command
/// position as an error and leaves the whole line sitting unexecuted at the
/// prompt.
pub async fn start_work_session(
    session: &str,
    dir: &Path,
    setup: &[String],
    skill: &str,
    issue: u64,
) -> Result<()> {
    let dir = dir.to_str().context("worktree path is not valid UTF-8")?;
    run_ok(&["new-session", "-d", "-s", session, "-c", dir]).await?;
    // The whole line is parsed by the shell; the quotes keep the `#<n>` from being
    // read as a comment.
    let claude = format!("claude \"Use the {skill} skill for issue #{issue}\"");
    let launch = if setup.is_empty() {
        claude
    } else {
        // Escape for a double-quoted string, which fish and POSIX shells parse
        // the same way (`\\`, `\"`, `\$`); `$` still expands, just inside `sh`.
        let script = setup
            .join(" && ")
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$");
        format!("sh -c \"{script}\" ; {claude}")
    };
    send_line(session, &launch).await?;
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
        anyhow::bail!(
            "`tmux {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}
