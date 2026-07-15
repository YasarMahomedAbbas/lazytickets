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
        };
        app.recompute(None);
        app
    }

    /// Title shown on the list pane.
    pub fn board_label(&self) -> String {
        format!("{} #{}", self.config.name, self.config.board.number)
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
}
