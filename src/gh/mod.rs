//! The only module that spawns `gh` subprocesses. Everything else works on the
//! typed structs in `crate::model`.

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
