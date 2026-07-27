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

/// Copy each `rel` path from the main checkout at `repo_root` into `worktree` at
/// the same relative location (creating parent dirs), seeding gitignored files a
/// fresh worktree lacks. A source that doesn't exist is skipped silently — the
/// config lists what a worktree *might* need, not what must be present. Returns
/// the relative paths actually copied.
pub fn seed_files(repo_root: &Path, worktree: &Path, rels: &[String]) -> Result<Vec<String>> {
    let mut copied = Vec::new();
    for rel in rels {
        let src = repo_root.join(rel);
        if !src.is_file() {
            continue;
        }
        let dst = worktree.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {} for the worktree", parent.display()))?;
        }
        std::fs::copy(&src, &dst).with_context(|| format!("copying {rel} into the worktree"))?;
        copied.push(rel.clone());
    }
    Ok(copied)
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

    #[test]
    fn seed_copies_present_sources_and_skips_missing() {
        // A throwaway repo/worktree pair under the temp dir. Fixed unique name so
        // the test stays deterministic (no rng); cleaned up at the end.
        let base = std::env::temp_dir().join("lazytickets_seed_test");
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("repo");
        let tree = base.join("worktree");
        std::fs::create_dir_all(root.join("Frontend")).unwrap();
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(root.join(".env"), "ROOT=1").unwrap();
        std::fs::write(root.join("Frontend/.env"), "FE=1").unwrap();

        let rels = vec![
            ".env".to_string(),
            "Frontend/.env".to_string(),
            "missing.env".to_string(), // skipped, not an error
        ];
        let copied = seed_files(&root, &tree, &rels).unwrap();

        assert_eq!(
            copied,
            vec![".env".to_string(), "Frontend/.env".to_string()]
        );
        assert_eq!(
            std::fs::read_to_string(tree.join(".env")).unwrap(),
            "ROOT=1"
        );
        // Nested parent dir was created under the worktree.
        assert_eq!(
            std::fs::read_to_string(tree.join("Frontend/.env")).unwrap(),
            "FE=1"
        );
        assert!(!tree.join("missing.env").exists());

        let _ = std::fs::remove_dir_all(&base);
    }
}
