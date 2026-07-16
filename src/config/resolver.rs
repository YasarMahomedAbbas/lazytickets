//! Map the current working directory's git repo to a configured project.
//!
//! Try in order, stop at first hit:
//!   1. explicit per-folder override (repo root path → project name)
//!   2. git-remote match (`origin` normalised to `owner/repo` → a project's `repos`)
//!   3. unknown → the caller runs the first-run wizard.
//!
//! `normalize_remote` is the pure, unit-tested core; the rest shells `git`.

use super::Config;
use super::schema::ProjectConfig;
use std::path::Path;
use std::process::Command;

/// Outcome of resolving the cwd against the config.
pub enum Resolution {
    /// A configured project matched — open its board directly. Boxed to keep the
    /// enum small (the `Unknown` variant is tiny).
    Project(Box<ProjectConfig>),
    /// No match. `repo` is the normalised `owner/name` if this is a GitHub repo,
    /// else `None` (not a git repo / no GitHub remote). The wizard uses it.
    Unknown { repo: Option<String> },
}

/// Normalise a git remote URL to `owner/repo`, or `None` if it isn't a
/// recognisable GitHub remote. Handles scp-style, https, and ssh:// forms.
pub fn normalize_remote(url: &str) -> Option<String> {
    let url = url.trim();

    // Isolate the "github.com<sep>owner/repo" tail, dropping any scheme/credentials.
    let rest = if let Some(idx) = url.find("github.com") {
        // After "github.com" comes either ':' (scp-style) or '/' (url path).
        let after = &url[idx + "github.com".len()..];
        after.trim_start_matches([':', '/'])
    } else {
        return None;
    };

    // rest is now "owner/repo(.git)(/...)". Take the first two path segments.
    let mut parts = rest.split('/').filter(|s| !s.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);

    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Resolve the repo at `cwd` to a project, or report it unknown.
pub fn resolve(config: &Config, cwd: &Path) -> Resolution {
    let root = git_toplevel(cwd);

    // 1. Path override on the repo root (or the cwd itself as a fallback).
    for candidate in [root.as_deref(), Some(cwd)].into_iter().flatten() {
        if let Some(name) = candidate.to_str().and_then(|p| config.overrides.get(p))
            && let Some(project) = config.find_project(name)
        {
            return Resolution::Project(Box::new(project.clone()));
        }
    }

    // 2. Git-remote match.
    let repo = root
        .as_deref()
        .and_then(git_origin_url)
        .as_deref()
        .and_then(normalize_remote);

    if let Some(repo) = &repo
        && let Some(project) = config
            .projects
            .iter()
            .find(|p| p.repos.iter().any(|r| r.eq_ignore_ascii_case(repo)))
    {
        return Resolution::Project(Box::new(project.clone()));
    }

    // 3. Unknown — hand the repo (if any) to the wizard.
    Resolution::Unknown { repo }
}

/// The normalised `owner/repo` for the git repo at `cwd`, or `None` if it isn't a
/// GitHub checkout. Used by the in-TUI "Add a board…" flow to feed the wizard.
pub fn repo_at(cwd: &Path) -> Option<String> {
    let root = git_toplevel(cwd)?;
    git_origin_url(&root).as_deref().and_then(normalize_remote)
}

/// `git -C <dir> rev-parse --show-toplevel` → repo root, or `None`.
fn git_toplevel(dir: &Path) -> Option<std::path::PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| std::path::PathBuf::from(path))
}

/// `git -C <root> remote get-url origin` → the raw URL, or `None`.
fn git_origin_url(root: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8(out.stdout).ok()?;
    let url = url.trim();
    (!url.is_empty()).then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_remote_url_variants() {
        let cases = [
            (
                "git@github.com:WhiteWolfStudio/travel-smart.git",
                "WhiteWolfStudio/travel-smart",
            ),
            (
                "git@github.com:WhiteWolfStudio/travel-smart",
                "WhiteWolfStudio/travel-smart",
            ),
            (
                "https://github.com/WhiteWolfStudio/travel-smart.git",
                "WhiteWolfStudio/travel-smart",
            ),
            (
                "https://github.com/WhiteWolfStudio/travel-smart",
                "WhiteWolfStudio/travel-smart",
            ),
            (
                "ssh://git@github.com/WhiteWolfStudio/travel-smart.git",
                "WhiteWolfStudio/travel-smart",
            ),
            (
                "https://github.com/YasarMahomedAbbas/lazytickets.git",
                "YasarMahomedAbbas/lazytickets",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                normalize_remote(input).as_deref(),
                Some(expected),
                "input: {input}"
            );
        }
    }

    /// End-to-end against the real repo these tests run in: git toplevel + origin
    /// are read for real. Robust to the exact owner/name via `discover`.
    fn discover() -> Option<String> {
        let cwd = std::env::current_dir().unwrap();
        git_origin_url(&git_toplevel(&cwd)?)
            .as_deref()
            .and_then(normalize_remote)
    }

    #[test]
    fn resolves_via_git_remote_and_override() {
        let Some(repo) = discover() else {
            return; // not a git checkout with an origin — skip
        };
        let cwd = std::env::current_dir().unwrap();

        // Empty config → Unknown, carrying the discovered repo.
        let empty = Config::default();
        match resolve(&empty, &cwd) {
            Resolution::Unknown { repo: Some(r) } => assert_eq!(r, repo),
            _ => panic!("empty config should be Unknown with the discovered repo"),
        }

        // A project listing this repo → matched by git-remote.
        let mut cfg = Config::default();
        cfg.projects.push(ProjectConfig::travel_smart());
        cfg.projects[0].repos = vec![repo.clone()];
        cfg.projects[0].name = "under-test".into();
        match resolve(&cfg, &cwd) {
            Resolution::Project(p) => assert_eq!(p.name, "under-test"),
            _ => panic!("git-remote match should resolve to the project"),
        }

        // A path override wins even without a repos entry.
        let root = git_toplevel(&cwd).unwrap();
        let mut cfg = Config::default();
        cfg.projects.push(ProjectConfig::travel_smart());
        cfg.projects[0].name = "override-target".into();
        cfg.projects[0].repos.clear();
        cfg.overrides.insert(
            root.to_string_lossy().into_owned(),
            "override-target".into(),
        );
        match resolve(&cfg, &cwd) {
            Resolution::Project(p) => assert_eq!(p.name, "override-target"),
            _ => panic!("path override should resolve to the project"),
        }
    }

    #[test]
    fn rejects_non_github_remotes() {
        assert_eq!(normalize_remote("git@gitlab.com:owner/repo.git"), None);
        assert_eq!(normalize_remote("https://example.com/owner/repo"), None);
        assert_eq!(normalize_remote(""), None);
        // github.com host but no repo segment.
        assert_eq!(normalize_remote("https://github.com/owner"), None);
    }
}
