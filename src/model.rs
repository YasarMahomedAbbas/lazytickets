//! Domain types. The `gh` wire format lives in the `gh` module; these are the
//! clean structs the rest of the app works on.

/// A single card on a GitHub Projects v2 board.
#[derive(Debug, Clone)]
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
}

impl Item {
    /// `#365` for issues, `—` for numberless drafts.
    pub fn number_label(&self) -> String {
        match self.number {
            Some(n) => format!("#{n}"),
            None => "—".to_string(),
        }
    }
}
