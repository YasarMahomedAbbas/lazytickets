//! Application state, selection, and the filtered view. Rendering lives in `ui`.

use crate::attach::{self, ContentPart};
use crate::config::schema::{Filter, Preset, ProjectConfig};
use crate::gh::issue::IssueDetail;
use crate::images::Images;
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
    /// Awaiting Enter/Esc before driving the claude pane. `prompt` is the line
    /// sent to Claude — prefilled from the skill + issue, editable in the modal.
    Confirm {
        item_id: String,
        issue: u64,
        skill: String,
        session: String,
        prompt: String,
    },
    /// Awaiting y/n before creating a worktree and starting the ticket in its own
    /// detached tmux session (`t`). `path` is the display path of the worktree;
    /// `base` is the display name of the branch it will fork from (so you can
    /// confirm you're on main); `base_rev` is the configured `worktree.base` when
    /// one is set, i.e. the explicit start point handed to `git worktree add`;
    /// `subdir` is the configured `claude_subdir` Claude boots inside, if any;
    /// `bootstrap` is a one-line summary of the seed/setup steps, if configured;
    /// `prompt` is the editable line handed to `claude` in the new session.
    WorktreeConfirm {
        item_id: String,
        issue: u64,
        skill: String,
        session: String,
        prompt: String,
        path: String,
        base: String,
        base_rev: Option<String>,
        subdir: Option<String>,
        bootstrap: Option<String>,
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
    /// Interactive builder for a new saved preset (name + status/label/assignee
    /// toggles). Persists to `config.toml` on save.
    FilterBuild(FilterDraft),
    /// New-ticket form: title + description, optional label + status. On submit
    /// creates a GitHub issue, adds it to the board, and (optionally) sets status.
    Create(CreateDraft),
    /// Awaiting y/n before deleting the preset at `index` (`name` for display).
    ConfirmDelete {
        index: usize,
        name: String,
    },
    /// A warning or result line; any key dismisses it.
    Message(String),
}

impl Modal {
    pub fn is_open(&self) -> bool {
        !matches!(self, Modal::None)
    }
}

/// In-progress state for the filter builder modal. The three option groups are
/// seeded from the values actually present on the board; `focus` walks a flat
/// list where `0` is the name field and `1..` are the option rows in order
/// (statuses, then labels, then assignees).
pub struct FilterDraft {
    pub name: String,
    pub statuses: Vec<(String, bool)>,
    pub labels: Vec<(String, bool)>,
    pub assignees: Vec<(String, bool)>,
    pub focus: usize,
    /// The name of the preset being edited, or `None` for a brand-new filter.
    /// On save, a rename (name changed from this) drops the old entry.
    pub original: Option<String>,
}

impl FilterDraft {
    /// Total number of toggleable option rows across all three groups.
    pub fn option_count(&self) -> usize {
        self.statuses.len() + self.labels.len() + self.assignees.len()
    }

    /// Move focus by `delta` over `[name, option_0, … option_n-1]`, clamped.
    pub fn move_focus(&mut self, delta: isize) {
        let max = self.option_count() as isize; // focus 0 = name, so max index = option_count
        self.focus = (self.focus as isize + delta).clamp(0, max) as usize;
    }

    /// Jump focus to the start of the previous/next section, where a section is
    /// the name field or a non-empty option group. Empty groups are skipped, and
    /// the ends clamp.
    pub fn jump_section(&mut self, delta: isize) {
        // Focus index at which each section begins: name (0), then the first
        // option row of each non-empty group.
        let mut starts = vec![0usize];
        let mut running = 1usize;
        for group in [&self.statuses, &self.labels, &self.assignees] {
            if !group.is_empty() {
                starts.push(running);
            }
            running += group.len();
        }
        let current = starts.iter().rposition(|&s| s <= self.focus).unwrap_or(0);
        let next = (current as isize + delta).clamp(0, starts.len() as isize - 1) as usize;
        self.focus = starts[next];
    }

    /// Flip the checkbox under the focused option row (no-op on the name field).
    pub fn toggle_focused(&mut self) {
        if self.focus == 0 {
            return;
        }
        let mut i = self.focus - 1;
        for group in [&mut self.statuses, &mut self.labels, &mut self.assignees] {
            if i < group.len() {
                group[i].1 = !group[i].1;
                return;
            }
            i -= group.len();
        }
    }

    /// Build a named `Preset` from the current selections, or `None` if the name
    /// is blank. Unchecked groups leave that filter dimension unconstrained.
    pub fn to_preset(&self) -> Option<Preset> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        let picked = |group: &[(String, bool)]| -> Vec<String> {
            group
                .iter()
                .filter(|(_, on)| *on)
                .map(|(v, _)| v.clone())
                .collect()
        };
        Some(Preset {
            name: name.to_string(),
            include: Filter {
                labels: picked(&self.labels),
                statuses: picked(&self.statuses),
                assignees: picked(&self.assignees),
            },
        })
    }
}

/// In-progress state for the create-ticket modal. `focus` walks a flat list of
/// fields: `0` title, `1` description, `2` label, `3` status, `4` the Create
/// button. `label_idx`/`status_idx` are 0 for "(none)", else 1-based into their
/// option lists (so cycling wraps through none → each option → none).
pub struct CreateDraft {
    /// `owner/name` the issue is created in (first configured repo).
    pub repo: String,
    pub title: String,
    pub body: String,
    /// Selectable labels present on the board; empty is allowed.
    pub labels: Vec<String>,
    pub label_idx: usize,
    /// Selectable statuses (config `status_order`, else board-distinct).
    pub statuses: Vec<String>,
    pub status_idx: usize,
    pub focus: usize,
}

impl CreateDraft {
    /// Highest focus index: title, description, label, status, then the button.
    pub const FOCUS_MAX: usize = 4;

    /// Move focus over `[title, body, label, status, create]`, clamped.
    pub fn move_focus(&mut self, delta: isize) {
        self.focus = (self.focus as isize + delta).clamp(0, Self::FOCUS_MAX as isize) as usize;
    }

    /// Cycle the label choice through `none → each label → none`.
    pub fn cycle_label(&mut self, delta: isize) {
        let n = self.labels.len() as isize;
        self.label_idx = (self.label_idx as isize + delta).rem_euclid(n + 1) as usize;
    }

    /// Cycle the status choice through `none → each status → none`.
    pub fn cycle_status(&mut self, delta: isize) {
        let n = self.statuses.len() as isize;
        self.status_idx = (self.status_idx as isize + delta).rem_euclid(n + 1) as usize;
    }

    /// The chosen label, or `None` for "(none)".
    pub fn chosen_label(&self) -> Option<&str> {
        (self.label_idx > 0).then(|| self.labels[self.label_idx - 1].as_str())
    }

    /// The chosen status, or `None` for "(none)".
    pub fn chosen_status(&self) -> Option<&str> {
        (self.status_idx > 0).then(|| self.statuses[self.status_idx - 1].as_str())
    }

    /// Display string for the label field.
    pub fn label_display(&self) -> &str {
        self.chosen_label().unwrap_or("(none)")
    }

    /// Display string for the status field.
    pub fn status_display(&self) -> &str {
        self.chosen_status().unwrap_or("(none)")
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
    /// Whether the list shows each card's labels inline. Toggled with `l`; a view
    /// preference, so it survives board switches.
    pub show_labels: bool,
    /// Inline-image state (picker + decoded/encoded cache) for the detail pane.
    pub images: Images,
    /// The current issue body split into text/image runs, for inline rendering.
    /// Rebuilt whenever a detail loads; empty when nothing is loaded.
    pub detail_parts: Vec<ContentPart>,
    /// Row scroll offset of the detail pane; reset to 0 on each new selection and
    /// clamped to the content height at render time.
    pub detail_scroll: u16,
    /// Height (rows) of the detail pane's inner content area, recorded each render.
    /// Drives the half-page Ctrl+D/Ctrl+U scroll; 0 until the first frame.
    pub detail_view_height: u16,
}

/// The default start-work prompt for a ticket: deterministic, no trigger-guessing.
pub fn default_prompt(skill: &str, issue: u64) -> String {
    format!("Use the {skill} skill for issue #{issue}")
}

impl Modal {
    /// The editable start-work prompt, when this modal carries one.
    pub fn prompt_mut(&mut self) -> Option<&mut String> {
        match self {
            Modal::Confirm { prompt, .. } | Modal::WorktreeConfirm { prompt, .. } => Some(prompt),
            _ => None,
        }
    }
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
            show_labels: false,
            images: Images::new(None),
            detail_parts: Vec::new(),
            detail_scroll: 0,
            detail_view_height: 0,
        };
        app.recompute(None);
        app
    }

    /// Flip inline labels in the list on/off.
    pub fn toggle_labels(&mut self) {
        self.show_labels = !self.show_labels;
    }

    /// Show a freshly-loaded issue: cache nothing here, just move it into the
    /// detail state and (re)derive the text/image runs the pane renders.
    pub fn show_detail(&mut self, d: IssueDetail) {
        self.detail_parts = attach::split_content(&d.body);
        self.detail = DetailState::Loaded(d);
    }

    /// Scroll the detail pane by `delta` rows (negative = up). Clamped to 0 here;
    /// the upper bound is applied at render time, where the content height is known.
    pub fn scroll_detail(&mut self, delta: i32) {
        self.detail_scroll = (self.detail_scroll as i32 + delta).max(0) as u16;
    }

    /// Scroll the detail pane by half its visible height (vim `C-d`/`C-u`);
    /// `dir` is +1 for down, -1 for up. At least one row before the first frame
    /// records a real height.
    pub fn scroll_detail_half(&mut self, dir: i32) {
        let half = (self.detail_view_height / 2).max(1) as i32;
        self.scroll_detail(dir * half);
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
        self.detail_parts.clear();
        self.detail_scroll = 0;
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

    // --- create ticket ---

    /// Open the create-ticket form, seeded with the board's labels and statuses.
    /// The issue is created in the first configured repo (falling back to a repo
    /// seen on the board). Returns false if no repo can be determined — the caller
    /// shows a message instead.
    pub fn open_create(&mut self) -> bool {
        let repo = self
            .config
            .repos
            .first()
            .cloned()
            .or_else(|| self.items.iter().find_map(|it| it.repository.clone()));
        let Some(repo) = repo else {
            return false;
        };
        let labels = self.distinct(|it| &it.labels);
        let statuses = if self.config.status_order.is_empty() {
            self.board_statuses()
        } else {
            self.config.status_order.clone()
        };
        self.modal = Modal::Create(CreateDraft {
            repo,
            title: String::new(),
            body: String::new(),
            labels,
            label_idx: 0,
            statuses,
            status_idx: 0,
            focus: 0,
        });
        true
    }

    // --- filter builder ---

    /// Open the builder for a brand-new filter: every board value present but
    /// unchecked, no name.
    pub fn open_filter_builder(&mut self) {
        self.modal = Modal::FilterBuild(FilterDraft {
            name: String::new(),
            statuses: Self::seed_options(self.board_statuses(), &[]),
            labels: Self::seed_options(self.distinct(|it| &it.labels), &[]),
            assignees: Self::seed_options(self.distinct(|it| &it.assignees), &[]),
            focus: 0,
            original: None,
        });
    }

    /// Open the builder pre-seeded from the active preset, so it can be edited
    /// in place. Values the preset references but that no board item currently
    /// has are still shown (and checked) so editing never silently drops them.
    pub fn open_filter_editor(&mut self) {
        let preset = &self.config.presets[self.active_preset];
        let name = preset.name.clone();
        let inc = preset.include.clone();
        let statuses = Self::seed_options(self.board_statuses(), &inc.statuses);
        let labels = Self::seed_options(self.distinct(|it| &it.labels), &inc.labels);
        let assignees = Self::seed_options(self.distinct(|it| &it.assignees), &inc.assignees);
        self.modal = Modal::FilterBuild(FilterDraft {
            name: name.clone(),
            statuses,
            labels,
            assignees,
            focus: 0,
            original: Some(name),
        });
    }

    /// Build a checkbox group from the board's values, ticking any that appear in
    /// `selected`, then appending selected values the board doesn't have (ticked).
    fn seed_options(board: Vec<String>, selected: &[String]) -> Vec<(String, bool)> {
        let mut opts: Vec<(String, bool)> = board
            .into_iter()
            .map(|v| {
                let on = selected.iter().any(|s| s.eq_ignore_ascii_case(&v));
                (v, on)
            })
            .collect();
        for s in selected {
            if !opts.iter().any(|(v, _)| v.eq_ignore_ascii_case(s)) {
                opts.push((s.clone(), true));
            }
        }
        opts
    }

    /// Distinct statuses on the board, ordered by `status_order` (unknowns last).
    fn board_statuses(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for it in &self.items {
            if let Some(s) = it.status.as_deref()
                && !seen.iter().any(|e| e.eq_ignore_ascii_case(s))
            {
                seen.push(s.to_string());
            }
        }
        seen.sort_by_key(|s| self.config.status_rank(Some(s)));
        seen
    }

    /// Distinct values of a `Vec<String>` field across every item, first-seen order.
    fn distinct(&self, field: impl Fn(&Item) -> &Vec<String>) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for it in &self.items {
            for v in field(it) {
                if !seen.iter().any(|e| e.eq_ignore_ascii_case(v)) {
                    seen.push(v.clone());
                }
            }
        }
        seen
    }

    /// Adopt `preset` into the in-memory config, make it active, and rebuild the
    /// view. `replacing` is the pre-edit name (from `FilterDraft::original`): if
    /// the name changed, the old entry is dropped so a rename doesn't duplicate.
    /// Otherwise a same-named preset is overwritten in place, else appended.
    /// Returns the index it landed at. Disk persistence is the caller's job.
    pub fn save_preset(&mut self, preset: Preset, replacing: Option<String>) -> usize {
        let keep = self.selected_id();
        if let Some(orig) = replacing
            && !orig.eq_ignore_ascii_case(&preset.name)
        {
            self.config
                .presets
                .retain(|p| !p.name.eq_ignore_ascii_case(&orig));
        }
        let idx = match self
            .config
            .presets
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(&preset.name))
        {
            Some(i) => {
                self.config.presets[i] = preset;
                i
            }
            None => {
                self.config.presets.push(preset);
                self.config.presets.len() - 1
            }
        };
        self.active_preset = idx;
        self.recompute(keep);
        idx
    }

    /// Remove the preset at `index`, clamp the active selection, and rebuild the
    /// view. Refuses to remove the last preset (the list must stay non-empty, as
    /// `recompute` always indexes one). Returns whether a preset was removed.
    pub fn delete_preset(&mut self, index: usize) -> bool {
        if self.config.presets.len() <= 1 || index >= self.config.presets.len() {
            return false;
        }
        let keep = self.selected_id();
        self.config.presets.remove(index);
        if self.active_preset >= self.config.presets.len() {
            self.active_preset = self.config.presets.len() - 1;
        }
        self.recompute(keep);
        true
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
    fn create_draft_cycles_optional_choices_and_clamps_focus() {
        let mut d = CreateDraft {
            repo: "o/r".into(),
            title: String::new(),
            body: String::new(),
            labels: vec!["Frontend".into(), "Backend".into()],
            label_idx: 0,
            statuses: vec!["Refine".into()],
            status_idx: 0,
            focus: 0,
        };
        // Default is "(none)".
        assert_eq!(d.chosen_label(), None);
        assert_eq!(d.status_display(), "(none)");

        // Forward cycles none → each option → none.
        d.cycle_label(1);
        assert_eq!(d.chosen_label(), Some("Frontend"));
        d.cycle_label(1);
        assert_eq!(d.chosen_label(), Some("Backend"));
        d.cycle_label(1);
        assert_eq!(d.chosen_label(), None, "wraps back to none");
        // Backward wraps to the last option.
        d.cycle_label(-1);
        assert_eq!(d.chosen_label(), Some("Backend"));

        // A single status option cycles none ↔ it.
        d.cycle_status(1);
        assert_eq!(d.chosen_status(), Some("Refine"));

        // Focus clamps to [0, FOCUS_MAX].
        d.move_focus(-1);
        assert_eq!(d.focus, 0);
        d.move_focus(99);
        assert_eq!(d.focus, CreateDraft::FOCUS_MAX);
    }

    #[test]
    fn open_create_seeds_repo_and_options() {
        let mut app = App::new(vec![item("a", "Refine")], ProjectConfig::travel_smart());
        assert!(app.open_create(), "a configured repo lets the form open");
        match &app.modal {
            Modal::Create(d) => {
                assert_eq!(d.repo, "WhiteWolfStudio/travel-smart");
                // Statuses come from config `status_order`.
                assert_eq!(d.statuses.first().map(String::as_str), Some("Refine"));
            }
            _ => panic!("create modal should be open"),
        }
    }

    #[test]
    fn filter_draft_builds_preset_from_checked_options() {
        let mut draft = FilterDraft {
            name: "   ".into(),
            statuses: vec![("Refine".into(), true), ("Done".into(), false)],
            labels: vec![("Frontend".into(), true)],
            assignees: vec![("me".into(), false)],
            focus: 0,
            original: None,
        };
        // A blank name yields no preset.
        assert!(draft.to_preset().is_none());

        draft.name = "backend-mine".into();
        let p = draft.to_preset().expect("named draft builds a preset");
        assert_eq!(p.name, "backend-mine");
        assert_eq!(p.include.statuses, vec!["Refine".to_string()]);
        assert_eq!(p.include.labels, vec!["Frontend".to_string()]);
        assert!(
            p.include.assignees.is_empty(),
            "unchecked group stays empty"
        );
    }

    #[test]
    fn filter_draft_toggle_and_focus_walk_groups() {
        let mut draft = FilterDraft {
            name: String::new(),
            statuses: vec![("s0".into(), false)],
            labels: vec![("l0".into(), false)],
            assignees: vec![("a0".into(), false)],
            focus: 0,
            original: None,
        };
        // focus 0 is the name field: toggling does nothing.
        draft.toggle_focused();
        assert!(!draft.statuses[0].1);

        // focus 2 = second option row = first label; focus 3 = the assignee.
        draft.focus = 2;
        draft.toggle_focused();
        assert!(draft.labels[0].1);
        draft.focus = 3;
        draft.toggle_focused();
        assert!(draft.assignees[0].1);

        // move_focus clamps to [0, option_count].
        draft.focus = 0;
        draft.move_focus(-1);
        assert_eq!(draft.focus, 0);
        draft.move_focus(99);
        assert_eq!(draft.focus, draft.option_count());
    }

    #[test]
    fn filter_draft_jump_section_skips_empty_groups() {
        let mut draft = FilterDraft {
            name: String::new(),
            statuses: vec![("s0".into(), false), ("s1".into(), false)],
            labels: vec![], // empty — jumps skip it
            assignees: vec![("a0".into(), false)],
            focus: 0,
            original: None,
        };
        // Section starts: name=0, status=1, assignees=3 (labels absent).
        draft.jump_section(1);
        assert_eq!(draft.focus, 1, "name → first status");
        draft.jump_section(1);
        assert_eq!(draft.focus, 3, "status → assignees, skipping empty labels");
        draft.jump_section(1);
        assert_eq!(draft.focus, 3, "clamps at the last section");

        // From the middle of a group, back-jump lands on that group's start first.
        draft.focus = 2; // second status row
        draft.jump_section(-1);
        assert_eq!(draft.focus, 0, "mid-status → name");
    }

    #[test]
    fn save_preset_replaces_by_name_and_activates() {
        let mut app = App::new(vec![item("a", "Refine")], ProjectConfig::travel_smart());
        let before = app.preset_count();

        // A new preset appends and becomes active.
        let idx = app.save_preset(
            Preset {
                name: "custom".into(),
                include: Filter::default(),
            },
            None,
        );
        assert_eq!(idx, before);
        assert_eq!(app.active_preset, idx);
        assert_eq!(app.preset_count(), before + 1);

        // Re-saving under the same name (case-insensitive) replaces in place.
        let idx2 = app.save_preset(
            Preset {
                name: "CUSTOM".into(),
                include: Filter {
                    statuses: vec!["Refine".into()],
                    ..Default::default()
                },
            },
            None,
        );
        assert_eq!(idx2, idx, "same name replaces, not appends");
        assert_eq!(app.preset_count(), before + 1);
        assert_eq!(
            app.config.presets[idx].include.statuses,
            vec!["Refine".to_string()]
        );
    }

    #[test]
    fn save_preset_rename_drops_the_old_entry() {
        let mut app = App::new(vec![item("a", "Refine")], ProjectConfig::travel_smart());
        let start = app.preset_count();
        let idx = app.save_preset(
            Preset {
                name: "draft".into(),
                include: Filter::default(),
            },
            None,
        );
        assert_eq!(app.preset_count(), start + 1);

        // Editing "draft" → "final" replaces in place, not adds a duplicate.
        app.save_preset(
            Preset {
                name: "final".into(),
                include: Filter::default(),
            },
            Some("draft".into()),
        );
        assert_eq!(app.preset_count(), start + 1, "rename doesn't add an entry");
        assert!(
            !app.config.presets.iter().any(|p| p.name == "draft"),
            "old name is gone"
        );
        assert!(app.config.presets.iter().any(|p| p.name == "final"));
        let _ = idx;
    }

    #[test]
    fn delete_preset_removes_and_guards_last() {
        let mut app = App::new(vec![item("a", "Refine")], ProjectConfig::travel_smart());
        let start = app.preset_count();
        assert!(start > 1, "fixture has several presets");

        // Delete every preset but one; the final delete is refused.
        for _ in 0..start {
            app.delete_preset(app.active_preset);
        }
        assert_eq!(app.preset_count(), 1, "can't delete the last preset");
        assert!(!app.delete_preset(0), "refusing returns false");
        assert!(
            app.active_preset < app.preset_count(),
            "selection stays valid"
        );
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
