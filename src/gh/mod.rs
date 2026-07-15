//! The only module that spawns `gh` subprocesses. Everything else works on the
//! typed structs in `crate::model`.

pub mod issue;
pub mod project;

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

/// The authenticated user's login, for seeding the `mine` preset in the wizard.
pub async fn viewer_login() -> Result<String> {
    let bytes = run(&["api", "user", "--jq", ".login"]).await?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}
