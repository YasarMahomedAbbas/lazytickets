//! Application state and selection logic. Rendering lives in `ui`.

use crate::gh::issue::IssueDetail;
use crate::model::Item;
use ratatui::widgets::ListState;

/// State of the right-hand detail pane for the current selection.
pub enum DetailState {
    /// No items, or nothing selected.
    Empty,
    /// Selected item is a draft (no issue number to fetch).
    Draft,
    /// Fetch in flight.
    Loading,
    Loaded(IssueDetail),
    Error(String),
}

pub struct App {
    pub items: Vec<Item>,
    pub list_state: ListState,
    /// Human label for the active board, shown in the list title.
    pub board_label: String,
    pub detail: DetailState,
}

impl App {
    pub fn new(items: Vec<Item>, board_label: String) -> Self {
        let mut list_state = ListState::default();
        if !items.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            items,
            list_state,
            board_label,
            detail: DetailState::Empty,
        }
    }

    /// Move selection down one, clamped to the last item.
    /// Returns true if the selection actually changed.
    pub fn next(&mut self) -> bool {
        self.select_delta(1)
    }

    /// Move selection up one, clamped to the first item.
    /// Returns true if the selection actually changed.
    pub fn prev(&mut self) -> bool {
        self.select_delta(-1)
    }

    fn select_delta(&mut self, delta: isize) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let cur = self.list_state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.items.len() as isize - 1) as usize;
        if next as isize == cur {
            return false;
        }
        self.list_state.select(Some(next));
        true
    }

    pub fn selected(&self) -> Option<&Item> {
        self.list_state.selected().and_then(|i| self.items.get(i))
    }
}
