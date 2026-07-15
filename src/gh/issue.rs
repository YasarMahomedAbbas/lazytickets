//! `gh issue view <n> --json …` → `IssueDetail`.
//!
//! Note the shape differs from `project item-list`: here labels are objects
//! (`{name, …}`) and comments carry a nested `author.login`.

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct IssueDetail {
    pub title: String,
    pub body: String,
    /// "OPEN" / "CLOSED".
    pub state: String,
    pub labels: Vec<String>,
    pub url: String,
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone)]
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
}
