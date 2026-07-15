//! Application state and selection logic. Rendering lives in `ui`.

use crate::model::Item;
use ratatui::widgets::ListState;

pub struct App {
    pub items: Vec<Item>,
    pub list_state: ListState,
    /// Human label for the active board, shown in the list title.
    pub board_label: String,
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
        }
    }

    /// Move selection down one, clamped to the last item.
    pub fn next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) if i + 1 < self.items.len() => i + 1,
            Some(i) => i,
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    /// Move selection up one, clamped to the first item.
    pub fn prev(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    #[allow(dead_code)] // used from M2 (detail pane) onward
    pub fn selected(&self) -> Option<&Item> {
        self.list_state.selected().and_then(|i| self.items.get(i))
    }
}
