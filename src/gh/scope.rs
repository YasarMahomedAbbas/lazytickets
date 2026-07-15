//! Token scope check for status writes. Reading a board needs `read:project`;
//! writing a card's Status field needs the broader `project` scope. We check
//! once before the first write and, if it's missing, tell the user to run
//! `gh auth refresh -s project` (a one-time, interactive step we don't drive
//! from inside the TUI).

use anyhow::{Context, Result};
use tokio::process::Command;

/// Whether the gh token carries the `project` (write) scope.
pub async fn has_project_scope() -> Result<bool> {
    let out = Command::new("gh")
        .args(["auth", "status"])
        .output()
        .await
        .context("spawning `gh auth status`")?;
    // gh prints the status (incl. the scopes line) to stderr.
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(scopes(&text).iter().any(|s| s == "project"))
}

/// The scope tokens from a `gh auth status` "Token scopes:" line. Pure so it's
/// unit-testable against captured output.
fn scopes(text: &str) -> Vec<String> {
    for line in text.lines() {
        if line.contains("Token scopes") {
            // Tokens are single-quoted: 'gist', 'read:project', 'project', …
            return line.split('\'').skip(1).step_by(2).map(str::to_string).collect();
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_project_is_not_project() {
        // read:project must NOT be mistaken for the write scope.
        let text = "  - Token scopes: 'gist', 'read:org', 'read:project', 'repo', 'workflow'";
        let s = scopes(text);
        assert!(s.iter().any(|x| x == "read:project"));
        assert!(!s.iter().any(|x| x == "project"));
    }

    #[test]
    fn detects_project_scope() {
        let text = "  - Token scopes: 'gist', 'project', 'read:project', 'repo'";
        assert!(scopes(text).iter().any(|x| x == "project"));
    }

    #[test]
    fn no_scopes_line() {
        assert!(scopes("not logged in").is_empty());
    }
}
