//! `gh project item-list` → `Vec<Item>`.

use crate::model::Item;
use anyhow::{Context, Result};
use serde::Deserialize;

/// Wire format of `gh project item-list --format json`. Kept private — callers
/// only ever see `model::Item`.
#[derive(Deserialize)]
struct RawList {
    items: Vec<RawItem>,
}

#[derive(Deserialize)]
struct RawItem {
    id: String,
    title: String,
    status: Option<String>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    assignees: Vec<String>,
    content: Option<RawContent>,
}

/// `number`, `url` and `repository` (`owner/name`) live nested under `content`
/// (and are absent for drafts).
#[derive(Deserialize)]
struct RawContent {
    number: Option<u64>,
    url: Option<String>,
    repository: Option<String>,
}

fn parse(bytes: &[u8]) -> Result<Vec<Item>> {
    let raw: RawList = serde_json::from_slice(bytes).context("parsing gh project item-list JSON")?;
    Ok(raw
        .items
        .into_iter()
        .map(|r| Item {
            id: r.id,
            title: r.title,
            status: r.status,
            labels: r.labels,
            assignees: r.assignees,
            number: r.content.as_ref().and_then(|c| c.number),
            repository: r.content.as_ref().and_then(|c| c.repository.clone()),
            url: r.content.and_then(|c| c.url),
        })
        .collect())
}

/// Fetch every card on a board. `--limit 200` comfortably covers current boards
/// (travel-smart #6 is ~66); paging is a later concern if a board outgrows it.
pub async fn item_list(owner: &str, number: u32) -> Result<Vec<Item>> {
    let num = number.to_string();
    let bytes = super::run(&[
        "project",
        "item-list",
        &num,
        "--owner",
        owner,
        "--format",
        "json",
        "--limit",
        "200",
    ])
    .await?;
    parse(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_captured_board_fixture() {
        let bytes = include_bytes!("../../tests/fixtures/board.json");
        let items = parse(bytes).expect("fixture should parse");
        assert!(!items.is_empty());

        // Every item has an id and title.
        assert!(items.iter().all(|i| !i.id.is_empty()));
        assert!(items.iter().all(|i| !i.title.is_empty()));

        // At least one real issue carries a number sourced from `content`.
        assert!(items.iter().any(|i| i.number.is_some()));
    }
}
