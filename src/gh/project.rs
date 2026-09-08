//! `gh project item-list` → `Vec<Item>`.

use crate::model::{Item, ParentRef};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;

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
    let raw: RawList =
        serde_json::from_slice(bytes).context("parsing gh project item-list JSON")?;
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
            parent: None, // stitched on by `item_list_with_parents`
        })
        .collect())
}

/// A board as listed by `gh project list` — enough for the wizard's picker.
#[derive(Debug, Clone)]
pub struct BoardSummary {
    pub number: u32,
    pub title: String,
}

#[derive(Deserialize)]
struct RawBoardList {
    projects: Vec<RawBoard>,
}

#[derive(Deserialize)]
struct RawBoard {
    number: u32,
    #[serde(default)]
    title: String,
    #[serde(default)]
    closed: bool,
}

/// List the (open) Projects v2 boards owned by `owner`, for the first-run wizard.
pub async fn list_boards(owner: &str) -> Result<Vec<BoardSummary>> {
    let bytes = super::run(&["project", "list", "--owner", owner, "--format", "json"]).await?;
    let raw: RawBoardList =
        serde_json::from_slice(&bytes).context("parsing gh project list JSON")?;
    Ok(raw
        .projects
        .into_iter()
        .filter(|b| !b.closed)
        .map(|b| BoardSummary {
            number: b.number,
            title: b.title,
        })
        .collect())
}

#[derive(Deserialize)]
struct RawFieldList {
    fields: Vec<RawField>,
}

#[derive(Deserialize)]
struct RawField {
    #[serde(default)]
    name: String,
    #[serde(default)]
    options: Vec<RawOption>,
}

#[derive(Deserialize)]
struct RawOption {
    name: String,
}

/// The `Status` single-select field's options, in board column order. Empty if
/// the board has no field named "Status". Seeds a new project's `status_order`.
pub async fn status_options(owner: &str, number: u32) -> Result<Vec<String>> {
    let num = number.to_string();
    let bytes = super::run(&[
        "project",
        "field-list",
        &num,
        "--owner",
        owner,
        "--format",
        "json",
    ])
    .await?;
    let raw: RawFieldList =
        serde_json::from_slice(&bytes).context("parsing gh project field-list JSON")?;
    Ok(raw
        .fields
        .into_iter()
        .find(|f| f.name.eq_ignore_ascii_case("Status"))
        .map(|f| f.options.into_iter().map(|o| o.name).collect())
        .unwrap_or_default())
}

/// Cards fetched per board. Comfortably covers current boards; paging is a later
/// concern if one outgrows it.
const BOARD_LIMIT: usize = 200;

/// Cards per page of the sub-issue query — must match `first:` in
/// `PARENT_QUERY`.
const PARENT_PAGE: usize = 100;

/// Fetch every card on a board, up to `BOARD_LIMIT`.
pub async fn item_list(owner: &str, number: u32) -> Result<Vec<Item>> {
    let num = number.to_string();
    let limit = BOARD_LIMIT.to_string();
    let bytes = super::run(&[
        "project",
        "item-list",
        &num,
        "--owner",
        owner,
        "--format",
        "json",
        "--limit",
        &limit,
    ])
    .await?;
    parse(&bytes)
}

/// The parent/sub-issue tree for a board, as `child → parent`.
///
/// `gh project item-list` doesn't carry the sub-issue relationship, so this is a
/// separate GraphQL query over the same items, asking only for each issue's
/// `parent`. Inverting `parent` is far cheaper than walking `subIssues` per card
/// and gives the same tree for anything actually *on* the board.
///
/// `$OWNER` is substituted with `organization` or `user`: `projectV2` hangs off
/// both, but GraphQL has no single root field covering the two, and only the
/// caller's board tells us which the owner is.
const PARENT_QUERY: &str = r#"
query($owner: String!, $number: Int!, $cursor: String) {
  $OWNER(login: $owner) {
    projectV2(number: $number) {
      items(first: 100, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          content {
            ... on Issue {
              number
              repository { nameWithOwner }
              parent { number repository { nameWithOwner } }
            }
          }
        }
      }
    }
  }
}"#;

#[derive(Deserialize)]
struct ParentResp {
    data: Option<ParentData>,
}

/// Only one of these is ever present — whichever root field the query asked for.
#[derive(Deserialize)]
struct ParentData {
    #[serde(default)]
    organization: Option<ProjectHolder>,
    #[serde(default)]
    user: Option<ProjectHolder>,
}

#[derive(Deserialize)]
struct ProjectHolder {
    #[serde(rename = "projectV2")]
    project: Option<ProjectItems>,
}

#[derive(Deserialize)]
struct ProjectItems {
    items: ItemsPage,
}

#[derive(Deserialize)]
struct ItemsPage {
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
    nodes: Vec<ParentNode>,
}

#[derive(Deserialize)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

/// `content` is `{}` for drafts and PRs — the `... on Issue` fragment simply
/// contributes nothing, so every field is optional.
#[derive(Deserialize)]
struct ParentNode {
    #[serde(default)]
    content: Option<ParentContent>,
}

#[derive(Deserialize)]
struct ParentContent {
    #[serde(default)]
    number: Option<u64>,
    #[serde(default)]
    repository: Option<RawRepo>,
    #[serde(default)]
    parent: Option<RawParent>,
}

#[derive(Deserialize)]
struct RawRepo {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

#[derive(Deserialize)]
struct RawParent {
    number: u64,
    repository: RawRepo,
}

/// One page of the parent query, folded into `out`. Returns the next cursor.
fn fold_page(bytes: &[u8], out: &mut HashMap<ParentRef, ParentRef>) -> Result<Option<String>> {
    let resp: ParentResp =
        serde_json::from_slice(bytes).context("parsing project sub-issue GraphQL JSON")?;
    let holder = resp
        .data
        .and_then(|d| d.organization.or(d.user))
        .and_then(|h| h.project);
    // A null owner/project means we asked the wrong root field — the caller
    // retries with the other one rather than treating it as an empty board.
    let Some(page) = holder.map(|p| p.items) else {
        anyhow::bail!("project not found under this owner kind");
    };

    for node in page.nodes {
        let Some(content) = node.content else {
            continue;
        };
        let (Some(number), Some(repo), Some(parent)) =
            (content.number, content.repository, content.parent)
        else {
            continue;
        };
        out.insert(
            ParentRef {
                repository: repo.name_with_owner,
                number,
            },
            ParentRef {
                repository: parent.repository.name_with_owner,
                number: parent.number,
            },
        );
    }
    Ok(page
        .page_info
        .has_next_page
        .then_some(page.page_info.end_cursor)
        .flatten())
}

/// Page through the parent query for one owner kind (`organization` / `user`).
async fn parent_links_as(
    owner_field: &str,
    owner: &str,
    number: u32,
) -> Result<HashMap<ParentRef, ParentRef>> {
    let query = PARENT_QUERY.replace("$OWNER", owner_field);
    // `-F` types its value (the Int the query declares); `-f` keeps a string, so
    // an all-digits login or cursor can't be coerced into a number.
    let num = format!("number={number}");
    let owner_arg = format!("owner={owner}");
    let query_arg = format!("query={query}");

    let mut out = HashMap::new();
    let mut cursor: Option<String> = None;
    // Stop at the same card count `item_list` returns: links past it point at
    // cards the board view never shows, so they're not worth an extra request.
    for _ in 0..BOARD_LIMIT.div_ceil(PARENT_PAGE) {
        let mut args = vec![
            "api", "graphql", "-f", &owner_arg, "-F", &num, "-f", &query_arg,
        ];
        let cursor_arg;
        if let Some(c) = &cursor {
            cursor_arg = format!("cursor={c}");
            args.push("-f");
            args.push(&cursor_arg);
        }
        let bytes = super::run(&args).await?;
        match fold_page(&bytes, &mut out)? {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(out)
}

/// The board's `child → parent` sub-issue links. Tries the owner as an org, then
/// as a user — `gh` exits non-zero on GraphQL's `NOT_FOUND` for the wrong one.
pub async fn parent_links(owner: &str, number: u32) -> Result<HashMap<ParentRef, ParentRef>> {
    match parent_links_as("organization", owner, number).await {
        Ok(links) => Ok(links),
        Err(_) => parent_links_as("user", owner, number).await,
    }
}

/// `item_list`, plus the sub-issue tree stitched onto each card. The tree drives
/// the parent badge in the list and the parent / sub-issue lines in the detail
/// pane in every view, so it's always requested — one extra GraphQL query per
/// board refresh, on a 30-minute cadence.
///
/// The parent query is **best-effort**: if it fails, the board still loads with
/// every card top-level, which is exactly the pre-sub-issue behaviour. The
/// returned flag says whether the tree really came back, so a parent-less
/// snapshot isn't cached as if it had one.
pub async fn item_list_with_parents(owner: &str, number: u32) -> Result<(Vec<Item>, bool)> {
    let mut items = item_list(owner, number).await?;
    let Ok(links) = parent_links(owner, number).await else {
        return Ok((items, false));
    };
    for it in &mut items {
        it.parent = it.key().and_then(|k| links.get(&k).cloned());
    }
    Ok((items, true))
}

#[derive(Deserialize)]
struct RawAddedItem {
    id: String,
}

/// Add an existing issue (by its web `url`) to board `(owner, number)`, returning
/// the new project item id (`PVTI_…`) — the handle a status write needs. Requires
/// the `project` write scope.
pub async fn add_item(owner: &str, number: u32, issue_url: &str) -> Result<String> {
    let num = number.to_string();
    let bytes = super::run(&[
        "project", "item-add", &num, "--owner", owner, "--url", issue_url, "--format", "json",
    ])
    .await?;
    let raw: RawAddedItem =
        serde_json::from_slice(&bytes).context("parsing gh project item-add JSON")?;
    Ok(raw.id)
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
        // Parents are stitched on separately, never by `item_list`.
        assert!(items.iter().all(|i| i.parent.is_none()));
    }

    fn parent_ref(number: u64) -> ParentRef {
        ParentRef {
            repository: "WhiteWolfStudio/travel-smart".to_string(),
            number,
        }
    }

    #[test]
    fn folds_captured_sub_issue_fixture() {
        let bytes = include_bytes!("../../tests/fixtures/sub_issues.json");
        let mut links = HashMap::new();
        let cursor = fold_page(bytes, &mut links).expect("fixture should parse");

        // Only sub-issues get an entry; top-level cards and the draft (whose
        // `content` is `{}`) contribute nothing.
        assert_eq!(links.len(), 3);
        assert_eq!(links.get(&parent_ref(1004)), Some(&parent_ref(1002)));
        assert_eq!(links.get(&parent_ref(1003)), Some(&parent_ref(1002)));
        assert_eq!(links.get(&parent_ref(1001)), Some(&parent_ref(999)));
        assert!(!links.contains_key(&parent_ref(423)));

        // `hasNextPage` hands back the cursor the next page resumes from.
        assert!(cursor.is_some());
    }

    #[test]
    fn fold_page_rejects_the_wrong_owner_kind() {
        // What GitHub returns when the board's owner is an org but we asked for
        // `user` — a null owner, not an empty board. Treating it as empty would
        // silently drop every link instead of retrying the other root field.
        let bytes = br#"{"data":{"user":null}}"#;
        let mut links = HashMap::new();
        assert!(fold_page(bytes, &mut links).is_err());
    }
}
