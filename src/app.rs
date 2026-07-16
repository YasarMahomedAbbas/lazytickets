//! Application state, selection, and the filtered view. Rendering lives in `ui`.

use crate::config::schema::ProjectConfig;
use crate::gh::issue::IssueDetail;
use crate::model::Item;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::widgets::ListState;
use std::collections::HashMap;

/// State of the right-hand detail pane for the current selection.
pub enum DetailState {
    Empty,
    Draft,
    Loading,
    Loaded(IssueDetail),
    Error(String),
}

/// Whether keystrokes drive navigation or the live filter input.
#[derive(PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Filter,
}

/// A modal overlay (start-work confirm, status mover, messages). Captures all
/// input while open.
pub enum Modal {
    None,
    /// Awaiting y/n before driving the claude pane.
    Confirm {
        item_id: String,
        issue: u64,
        skill: String,
        session: String,
    },
    /// Status column picker (M6). `selected` indexes `options`.
    StatusMove {
        item_id: String,
        options: Vec<String>,
        selected: usize,
    },
    /// Project switcher. `names` are the configured project names; `selected`
    /// ranges over `0..=names.len()`, where the trailing index is "Add a board…".
    ProjectPick {
        names: Vec<String>,
        selected: usize,
    },
    /// Keybindings overlay (M7); any key dismisses.
    Help,
    /// A warning or result line; any key dismisses it.
    Message(String),
}

impl Modal {
    pub fn is_open(&self) -> bool {
        !matches!(self, Modal::None)
    }
}

pub struct App {
    /// The full, unfiltered board.
    pub items: Vec<Item>,
    pub config: ProjectConfig,
    pub active_preset: usize,
    pub input_mode: InputMode,
    pub filter_query: String,
    /// Indices into `items`, after preset + exclude + fuzzy filtering, sorted by
    /// status order. This is what the list renders and selection indexes into.
    pub visible: Vec<usize>,
    pub list_state: ListState,
    pub detail: DetailState,
    pub detail_cache: HashMap<String, IssueDetail>,
    pub modal: Modal,
    /// Board Status field (ids + options), fetched lazily on the first write.
    pub status_field: Option<crate::gh::write::StatusField>,
    /// Whether the token has the `project` write scope; checked once.
    pub project_scope: Option<bool>,
}

impl App {
    pub fn new(items: Vec<Item>, config: ProjectConfig) -> Self {
        let mut app = Self {
            items,
            config,
            active_preset: 0,
            input_mode: InputMode::Normal,
            filter_query: String::new(),
            visible: Vec::new(),
            list_state: ListState::default(),
            detail: DetailState::Empty,
            detail_cache: HashMap::new(),
            modal: Modal::None,
            status_field: None,
            project_scope: None,
        };
        app.recompute(None);
        app
    }

    /// Title shown on the list pane.
    pub fn board_label(&self) -> String {
        format!("{} #{}", self.config.name, self.config.board.number)
    }

    /// Swap the active project: adopt a new config + freshly-fetched board and
    /// reset all per-board state (preset, filter, detail cache, status field).
    /// `project_scope` survives — it's a property of the token, not the board.
    pub fn switch_board(&mut self, config: ProjectConfig, items: Vec<Item>) {
        self.config = config;
        self.items = items;
        self.active_preset = 0;
        self.input_mode = InputMode::Normal;
        self.filter_query.clear();
        self.detail = DetailState::Empty;
        self.detail_cache.clear();
        self.status_field = None;
        self.modal = Modal::None;
        self.recompute(None);
    }

    /// Rebuild `visible` from the active preset, exclusions and fuzzy query,
    /// sorted by status order. Preserves the selected item (`keep_id`) if it
    /// survives the new filter, else clamps to the top.
    pub fn recompute(&mut self, keep_id: Option<String>) {
        let preset = &self.config.presets[self.active_preset];
        let matcher = SkimMatcherV2::default();
        let query = self.filter_query.trim();

        let mut idx: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| self.config.keeps(preset, it))
            .filter(|(_, it)| query.is_empty() || matcher.fuzzy_match(&it.title, query).is_some())
            .map(|(i, _)| i)
            .collect();

        idx.sort_by_key(|&i| self.config.status_rank(self.items[i].status.as_deref()));
        self.visible = idx;

        // Restore selection to the same item where possible.
        let new_sel = keep_id
            .and_then(|id| self.visible.iter().position(|&i| self.items[i].id == id))
            .or_else(|| (!self.visible.is_empty()).then_some(0));
        self.list_state.select(new_sel);
    }

    pub fn selected(&self) -> Option<&Item> {
        let vi = self.list_state.selected()?;
        self.visible.get(vi).map(|&i| &self.items[i])
    }

    fn selected_id(&self) -> Option<String> {
        self.selected().map(|i| i.id.clone())
    }

    pub fn next(&mut self) -> bool {
        self.select_delta(1)
    }

    pub fn prev(&mut self) -> bool {
        self.select_delta(-1)
    }

    fn select_delta(&mut self, delta: isize) -> bool {
        if self.visible.is_empty() {
            return false;
        }
        let cur = self.list_state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.visible.len() as isize - 1) as usize;
        if next as isize == cur {
            return false;
        }
        self.list_state.select(Some(next));
        true
    }

    // --- presets ---

    pub fn preset_name(&self, i: usize) -> &str {
        &self.config.presets[i].name
    }

    pub fn preset_count(&self) -> usize {
        self.config.presets.len()
    }

    /// Switch to preset `i`. Returns true if it changed (caller reschedules detail).
    pub fn set_preset(&mut self, i: usize) -> bool {
        if i >= self.config.presets.len() || i == self.active_preset {
            return false;
        }
        let keep = self.selected_id();
        self.active_preset = i;
        self.recompute(keep);
        true
    }

    pub fn cycle_preset(&mut self, delta: isize) -> bool {
        let n = self.config.presets.len() as isize;
        let next = (self.active_preset as isize + delta).rem_euclid(n) as usize;
        self.set_preset(next)
    }

    // --- live filter ---

    pub fn enter_filter(&mut self) {
        self.input_mode = InputMode::Filter;
    }

    /// Leave filter input but keep the query applied.
    pub fn confirm_filter(&mut self) {
        self.input_mode = InputMode::Normal;
    }

    /// Leave filter input and clear the query.
    pub fn cancel_filter(&mut self) {
        self.input_mode = InputMode::Normal;
        if !self.filter_query.is_empty() {
            self.filter_query.clear();
            let keep = self.selected_id();
            self.recompute(keep);
        }
    }

    pub fn push_filter(&mut self, c: char) {
        let keep = self.selected_id();
        self.filter_query.push(c);
        self.recompute(keep);
    }

    pub fn pop_filter(&mut self) {
        let keep = self.selected_id();
        self.filter_query.pop();
        self.recompute(keep);
    }

    // --- status mover (M6) ---

    /// Move the highlighted row in an open picker modal (`StatusMove` or
    /// `ProjectPick`) by `delta`.
    pub fn modal_move(&mut self, delta: isize) {
        match &mut self.modal {
            Modal::StatusMove {
                options, selected, ..
            } if !options.is_empty() => {
                let next =
                    (*selected as isize + delta).clamp(0, options.len() as isize - 1) as usize;
                *selected = next;
            }
            Modal::ProjectPick { names, selected } => {
                // The trailing row (index == names.len()) is the "Add a board…" entry.
                let max = names.len() as isize;
                *selected = (*selected as isize + delta).clamp(0, max) as usize;
            }
            _ => {}
        }
    }

    /// The (item id, chosen status) of an open `StatusMove` modal, if any.
    pub fn modal_status_pick(&self) -> Option<(String, String)> {
        match &self.modal {
            Modal::StatusMove {
                item_id,
                options,
                selected,
            } => options.get(*selected).map(|s| (item_id.clone(), s.clone())),
            _ => None,
        }
    }

    /// Set an item's status in place (optimistic update) and re-sort the view,
    /// keeping it selected. Returns the previous status for revert-on-failure.
    pub fn set_item_status(&mut self, item_id: &str, status: Option<String>) -> Option<String> {
        let item = self.items.iter_mut().find(|i| i.id == item_id)?;
        let old = item.status.take();
        item.status = status;
        self.recompute(Some(item_id.to_string()));
        old
    }

    /// The current status of an item, by id.
    pub fn item_status(&self, item_id: &str) -> Option<String> {
        self.items
            .iter()
            .find(|i| i.id == item_id)
            .and_then(|i| i.status.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::ProjectConfig;
    use crate::model::Item;

    fn item(id: &str, status: &str) -> Item {
        Item {
            id: id.into(),
            number: Some(1),
            title: format!("t{id}"),
            repository: Some("o/r".into()),
            status: Some(status.into()),
            labels: vec![],
            assignees: vec![],
            url: None,
        }
    }

    #[test]
    fn optimistic_status_update_and_revert() {
        let items = vec![item("a", "Refine"), item("b", "In progress")];
        let mut app = App::new(items, ProjectConfig::travel_smart());

        // Optimistic move returns the previous status for revert.
        let old = app.set_item_status("a", Some("Done".into()));
        assert_eq!(old.as_deref(), Some("Refine"));
        assert_eq!(app.item_status("a").as_deref(), Some("Done"));

        // Reconcile-on-failure restores it.
        app.set_item_status("a", old);
        assert_eq!(app.item_status("a").as_deref(), Some("Refine"));

        // Unknown ids are a no-op, not a panic.
        assert_eq!(app.set_item_status("missing", Some("x".into())), None);
    }

    #[test]
    fn project_pick_navigation_includes_add_row() {
        let mut app = App::new(vec![item("a", "Refine")], ProjectConfig::travel_smart());
        // Two projects → rows are [p0, p1, "Add a board…"], so the last index is 2.
        app.modal = Modal::ProjectPick {
            names: vec!["p0".into(), "p1".into()],
            selected: 0,
        };

        app.modal_move(1);
        app.modal_move(1);
        app.modal_move(1); // clamps at the trailing add-board row
        match &app.modal {
            Modal::ProjectPick { selected, names } => {
                assert_eq!(*selected, names.len(), "should rest on the add-board row");
            }
            _ => panic!("modal changed unexpectedly"),
        }

        app.modal_move(-5); // clamps at the top
        assert!(matches!(&app.modal, Modal::ProjectPick { selected: 0, .. }));
    }

    #[test]
    fn switch_board_resets_per_board_state() {
        let mut app = App::new(vec![item("a", "Refine")], ProjectConfig::travel_smart());
        app.filter_query = "x".into();
        app.active_preset = 1;
        app.detail_cache
            .insert("stale".into(), unreachable_detail());

        let mut other = ProjectConfig::travel_smart();
        other.name = "other".into();
        app.switch_board(other, vec![item("b", "In progress"), item("c", "Refine")]);

        assert_eq!(app.config.name, "other");
        assert_eq!(app.items.len(), 2);
        assert_eq!(app.active_preset, 0);
        assert!(app.filter_query.is_empty());
        assert!(app.detail_cache.is_empty());
        assert!(app.status_field.is_none());
        assert!(matches!(app.modal, Modal::None));
        assert!(
            app.selected().is_some(),
            "a selection is restored on the new board"
        );
    }

    /// A throwaway `IssueDetail` for the cache-clearing assertion above.
    fn unreachable_detail() -> IssueDetail {
        IssueDetail {
            title: String::new(),
            body: String::new(),
            state: String::new(),
            labels: vec![],
            url: String::new(),
            comments: vec![],
        }
    }
}
