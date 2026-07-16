//! The only module that spawns `gh` subprocesses. Everything else works on the
//! typed structs in `crate::model`.

pub mod issue;
pub mod project;
pub mod scope;
pub mod write;

use anyhow::{Context, Result};
use tokio::process::Command;

/// Run `gh` with the given args, capturing stdout. Errors on a non-zero exit,
/// surfacing gh's stderr.
pub async fn run(args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .await
        .context("failed to spawn `gh` — is the GitHub CLI installed and on PATH?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("`gh {}` failed: {}", args.join(" "), stderr.trim());
    }
    Ok(output.stdout)
}

/// Whether an error from `run` is GitHub refusing us for rate reasons. The
/// poller uses this to back off instead of retrying on schedule — continued
/// requests during a (secondary) rate-limit only extend its window.
pub fn is_rate_limit(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("rate limit") || msg.contains("abuse detection")
}

/// The authenticated user's login, for seeding the `mine` preset in the wizard.
pub async fn viewer_login() -> Result<String> {
    let bytes = run(&["api", "user", "--jq", ".login"]).await?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_rate_limit_errors() {
        // The shape `run` produces on a limited board.
        let e = anyhow::anyhow!(
            "`gh project item-list 3` failed: GraphQL: API rate limit exceeded for user ID 11193066"
        );
        assert!(is_rate_limit(&e));
        assert!(is_rate_limit(&anyhow::anyhow!(
            "You have exceeded a secondary rate limit"
        )));
        assert!(is_rate_limit(&anyhow::anyhow!(
            "triggered an abuse detection mechanism"
        )));
        // Unrelated failures must not be mistaken for a limit (they retry normally).
        assert!(!is_rate_limit(&anyhow::anyhow!(
            "could not resolve host github.com"
        )));
    }
}
