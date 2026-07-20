//! git worktree creation for the parallel "start in a worktree" flow (`t`).
//!
//! Each ticket gets its own worktree in a sibling `worktrees/` dir next to the
//! repo, on a branch named after the issue, driven by its own detached tmux
//! session (see `tmux::start_work_session`). Because every ticket is isolated,
//! there's no busy-guard — you can fire the whole backlog off in parallel.
//!
//! Spawns `git`, like `config::resolver`; the path derivation is pure and tested.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Branch (and tmux session) name for an issue's worktree, e.g. `issue-123`.
/// Stable, collision-free, and trivially mapped back to the ticket.
pub fn branch_for(issue: u64) -> String {
    format!("issue-{issue}")
}

/// Where an issue's worktree lives: a sibling `worktrees/` dir next to the repo,
/// i.e. `<repo_root>/../worktrees/issue-<n>`. `None` if the repo root is a
/// filesystem root with no parent.
pub fn worktree_path(repo_root: &Path, branch: &str) -> Option<PathBuf> {
    Some(repo_root.parent()?.join("worktrees").join(branch))
}

/// `git -C <cwd> rev-parse --show-toplevel` → the repo root, or `None` if `cwd`
/// isn't inside a git repository.
pub async fn repo_root(cwd: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// The branch currently checked out at `repo_root` — the base a new worktree
/// branches off. `None` in detached-HEAD state (or on error), which the caller
/// renders as a warning so you never blitz off a stray commit by accident.
pub async fn current_branch(repo_root: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["branch", "--show-current"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8(out.stdout).ok()?;
    let branch = branch.trim();
    (!branch.is_empty()).then(|| branch.to_string())
}

/// `git -C <repo_root> worktree add <path> -b <branch>`, branching off the
/// current HEAD. Errors (git refuses) if `branch` or `path` already exists —
/// which the caller surfaces as "this ticket's already been started".
pub async fn add(repo_root: &Path, path: &Path, branch: &str) -> Result<()> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["worktree", "add"])
        .arg(path)
        .arg("-b")
        .arg(branch)
        .output()
        .await
        .context("failed to spawn `git worktree add`")?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_name_from_issue() {
        assert_eq!(branch_for(123), "issue-123");
        assert_eq!(branch_for(1), "issue-1");
    }

    #[test]
    fn worktree_is_a_sibling_of_the_repo() {
        let root = Path::new("/home/dracul/projects/personal/lazyissues");
        let path = worktree_path(root, &branch_for(42)).unwrap();
        assert_eq!(
            path,
            Path::new("/home/dracul/projects/personal/worktrees/issue-42")
        );
    }

    #[test]
    fn worktree_path_needs_a_parent() {
        assert_eq!(worktree_path(Path::new("/"), "issue-1"), None);
    }
}
