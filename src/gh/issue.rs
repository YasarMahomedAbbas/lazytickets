//! `gh issue view <n> --json …` → `IssueDetail`.
//!
//! Note the shape differs from `project item-list`: here labels are objects
//! (`{name, …}`) and comments carry a nested `author.login`.

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IssueDetail {
    pub title: String,
    pub body: String,
    /// "OPEN" / "CLOSED".
    pub state: String,
    pub labels: Vec<String>,
    pub url: String,
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Comment {
    pub author: String,
    pub body: String,
}

#[derive(Deserialize)]
struct RawIssue {
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    labels: Vec<RawLabel>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    comments: Vec<RawComment>,
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
}

#[derive(Deserialize)]
struct RawComment {
    #[serde(default)]
    author: RawAuthor,
    #[serde(default)]
    body: String,
}

#[derive(Deserialize, Default)]
struct RawAuthor {
    /// Absent for deleted/ghost authors.
    #[serde(default)]
    login: String,
}

fn parse(bytes: &[u8]) -> Result<IssueDetail> {
    let raw: RawIssue = serde_json::from_slice(bytes).context("parsing gh issue view JSON")?;
    Ok(IssueDetail {
        title: raw.title,
        body: raw.body,
        state: raw.state,
        labels: raw.labels.into_iter().map(|l| l.name).collect(),
        url: raw.url,
        comments: raw
            .comments
            .into_iter()
            .map(|c| Comment {
                author: if c.author.login.is_empty() {
                    "ghost".to_string()
                } else {
                    c.author.login
                },
                body: c.body,
            })
            .collect(),
    })
}

/// Fetch a single issue's detail. `repo` is `owner/name`.
pub async fn view(repo: &str, number: u64) -> Result<IssueDetail> {
    let num = number.to_string();
    let bytes = super::run(&[
        "issue",
        "view",
        &num,
        "--repo",
        repo,
        "--json",
        "title,body,labels,comments,url,state",
    ])
    .await?;
    parse(&bytes)
}

/// Open an issue in the browser via `gh issue view --web`. `repo` is `owner/name`.
pub async fn open_web(repo: &str, number: u64) -> Result<()> {
    let num = number.to_string();
    super::run(&["issue", "view", &num, "--repo", repo, "--web"]).await?;
    Ok(())
}

/// A freshly-created issue: enough to add it to a board and report it back.
pub struct CreatedIssue {
    pub url: String,
    pub number: u64,
}

/// Create an issue in `repo` (`owner/name`) via `gh issue create`, optionally
/// tagging it with `labels` (each must already exist in the repo, else `gh`
/// refuses and nothing is created). `gh` prints the new issue's URL, from which
/// we recover the number.
pub async fn create(
    repo: &str,
    title: &str,
    body: &str,
    labels: &[String],
) -> Result<CreatedIssue> {
    let mut args: Vec<&str> = vec![
        "issue", "create", "--repo", repo, "--title", title, "--body", body,
    ];
    for l in labels {
        args.push("--label");
        args.push(l.as_str());
    }
    let out = super::run(&args).await?;

    let text = String::from_utf8_lossy(&out);
    let url = text
        .lines()
        .rev()
        .find(|l| l.contains("/issues/"))
        .unwrap_or("")
        .trim()
        .to_string();
    if url.is_empty() {
        anyhow::bail!("gh issue create did not print an issue URL");
    }
    let number = issue_number_from_url(&url)
        .context("could not parse the issue number from the created URL")?;
    Ok(CreatedIssue { url, number })
}

/// The trailing number in an issue URL (`…/issues/365` → `365`).
fn issue_number_from_url(url: &str) -> Option<u64> {
    url.trim_end_matches('/').rsplit('/').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_captured_issue_fixture() {
        let bytes = include_bytes!("../../tests/fixtures/issue.json");
        let d = parse(bytes).expect("fixture should parse");
        assert!(!d.title.is_empty());
        assert!(!d.state.is_empty());
        // Fixture #226 carries the "bug" label.
        assert!(d.labels.iter().any(|l| l == "bug"));
    }

    #[test]
    fn parses_issue_number_from_created_url() {
        assert_eq!(
            issue_number_from_url("https://github.com/o/r/issues/365"),
            Some(365)
        );
        assert_eq!(
            issue_number_from_url("https://github.com/o/r/issues/12/"),
            Some(12)
        );
        assert_eq!(
            issue_number_from_url("https://github.com/o/r/pull/9"),
            Some(9)
        );
        assert_eq!(issue_number_from_url("not-a-url"), None);
    }
}
