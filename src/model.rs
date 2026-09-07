//! Domain types. The `gh` wire format lives in the `gh` module; these are the
//! clean structs the rest of the app works on.

/// A single card on a GitHub Projects v2 board.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Item {
    /// Project item node id (`PVTI_…`) — stable handle for status writes (M6).
    #[allow(dead_code)] // read from M6 (status writes)
    pub id: String,
    /// Issue/PR number. `None` for draft items, which have no number.
    pub number: Option<u64>,
    pub title: String,
    /// `owner/name` of the item's repo, used to target `gh issue view --repo`.
    /// `None` for draft items.
    pub repository: Option<String>,
    /// Board Status column value, e.g. "In progress". `None` if unset.
    pub status: Option<String>,
    pub labels: Vec<String>,
    /// GitHub logins assigned to the item. Drives the `mine` preset.
    pub assignees: Vec<String>,
    /// Web URL. `None` for drafts.
    #[allow(dead_code)] // read from M2 (detail) / M7 (open in browser)
    pub url: Option<String>,
    /// The issue this card is a *sub-issue* of, when the board carries the
    /// parent/sub-issue tree. `None` for top-level cards, drafts, and every
    /// board fetched without the parent query (`serde(default)` so an older
    /// cached snapshot still deserialises).
    #[serde(default)]
    pub parent: Option<ParentRef>,
}

/// A reference to another issue by `owner/name` + number — the key shape used to
/// match a sub-issue's parent back to a card on the board. Sub-issues can be
/// cross-repo, so the number alone isn't a unique handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ParentRef {
    pub repository: String,
    pub number: u64,
}

impl Item {
    /// The `(repo, number)` handle this card is addressed by, for matching
    /// sub-issues to their parent. `None` for drafts (no number, no repo).
    pub fn key(&self) -> Option<ParentRef> {
        Some(ParentRef {
            repository: self.repository.clone()?,
            number: self.number?,
        })
    }

    /// `#365` for issues, `—` for numberless drafts.
    pub fn number_label(&self) -> String {
        match self.number {
            Some(n) => format!("#{n}"),
            None => "—".to_string(),
        }
    }
}
