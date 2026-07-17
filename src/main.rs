mod app;
mod cache;
mod config;
mod gh;
mod model;
mod poll;
mod tmux;
mod ui;

use app::{App, DetailState, InputMode, Modal};
use config::Config;
use config::resolver::{self, Resolution};
use model::Item;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

/// A completed detail fetch: the selection generation it was for, the item id it
/// belongs to (for the cache), and the result.
type DetailMsg = (u64, String, anyhow::Result<gh::issue::IssueDetail>);

/// A completed background status write: the item id, the status to revert to if
/// it failed, and the outcome.
type WriteMsg = (String, Option<String>, anyhow::Result<()>);

/// Shown when a status write is attempted without the `project` token scope.
const SCOPE_HINT: &str = "Status writes need the 'project' scope. Quit and run:\n  gh auth refresh -s project\nthen relaunch.";

/// Fail fast, with a plain-terminal message, when a runtime dependency is
/// missing — far friendlier than the first `gh`/`tmux` spawn erroring mid-TUI.
/// `gh` is required to load any board; `tmux` only for the start-work flow, so a
/// missing `tmux` is a warning, not a hard stop.
fn preflight() -> anyhow::Result<()> {
    let present = |bin: &str| {
        std::process::Command::new(bin)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    };

    if !present("gh") {
        anyhow::bail!(
            "`gh` (the GitHub CLI) was not found on PATH.\n\
             lazytickets needs it to read your board. Install it from https://cli.github.com \
             and run `gh auth login`, then relaunch."
        );
    }
    if !present("tmux") {
        eprintln!(
            "warning: `tmux` not found on PATH — the start-work flow (`s`) is unavailable until it is installed."
        );
    }
    Ok(())
}

/// Handle the trivial informational flags before spinning up tokio/the terminal.
/// Returns true if we printed something and `main` should exit. The app takes no
/// other arguments — it resolves the board from the current directory.
fn handle_flags() -> bool {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-V" | "--version" => {
                println!("lazytickets {}", env!("CARGO_PKG_VERSION"));
                return true;
            }
            "-h" | "--help" => {
                println!(
                    "lazytickets {} — a TUI for GitHub Projects v2 boards.\n\n\
                     Usage: lazytickets\n\n\
                     Run inside a tmux session, from within a git repo whose remote maps to a\n\
                     configured board. Press `?` in the app for keybindings.\n\n\
                     Requires `gh` (authenticated) and `tmux` on PATH.",
                    env!("CARGO_PKG_VERSION")
                );
                return true;
            }
            _ => {}
        }
    }
    false
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if handle_flags() {
        return Ok(());
    }
    preflight()?;

    // Resolve the current repo → its board (config), before touching the terminal.
    let config = Config::load()?;
    let cwd = std::env::current_dir()?;
    let resolution = resolver::resolve(&config, &cwd);

    let mut terminal = ratatui::init();

    // Known repo → open directly; unknown → the first-run wizard writes a config.
    let cfg = match resolution {
        Resolution::Project(p) => *p,
        Resolution::Unknown { repo } => match config::wizard::run(
            &mut terminal,
            repo,
            &cwd,
            config,
            true,
            config::wizard::Input::Terminal,
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                ratatui::restore();
                return Err(e);
            }
        },
    };

    let result = match load_board(&cfg.board.owner, cfg.board.number).await {
        Ok(items) => {
            let mut app = App::new(items, cfg);
            run(&mut terminal, &mut app).await
        }
        Err(e) => Err(e),
    };
    ratatui::restore();
    result
}

/// Load a board for display, minimising API traffic: serve a fresh-enough disk
/// snapshot without touching the network, otherwise fetch and re-cache it. If the
/// fetch fails but *any* cached snapshot exists (even stale), fall back to it — so
/// a rate-limited launch still shows the last-known board instead of an error.
async fn load_board(owner: &str, number: u32) -> anyhow::Result<Vec<Item>> {
    let cached = cache::load_board(owner, number);
    if let Some(c) = &cached
        && c.age() < cache::BOARD_TTL
    {
        return Ok(c.value.clone());
    }
    match gh::project::item_list(owner, number).await {
        Ok(items) => {
            cache::save_board(owner, number, &items);
            Ok(items)
        }
        Err(e) => match cached {
            Some(c) => Ok(c.value),
            None => Err(e),
        },
    }
}

async fn run(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    // Input: a blocking reader thread feeding an async channel, so key handling
    // never blocks the tokio runtime (busy with detail fetches).
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Event>();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if input_tx.send(ev).is_err() {
                break; // UI gone
            }
        }
    });

    // Detail fetches: debounced and generation-guarded.
    let (detail_tx, mut detail_rx) = mpsc::unbounded_channel::<DetailMsg>();
    let generation = Arc::new(AtomicU64::new(0));
    schedule_detail(app, &generation, &detail_tx);

    // Background status writes (optimistic; reconciled on completion).
    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<WriteMsg>();

    // Background board poll (external changes appear silently within ~45s) + the
    // `r` force-refresh, both delivering fresh snapshots on the same channel.
    // The handle is re-targeted when the user switches projects (`p`).
    let (poll_tx, mut poll_rx) = mpsc::unbounded_channel::<poll::Snapshot>();
    let mut poll_handle = poll::spawn(
        app.config.board.owner.clone(),
        app.config.board.number,
        poll_tx.clone(),
    );

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        tokio::select! {
            maybe_ev = input_rx.recv() => {
                let Some(ev) = maybe_ev else { break }; // input thread ended
                if let Event::Key(key) = ev
                    && key.kind == KeyEventKind::Press
                {
                    let mut reschedule = false;
                    if app.modal.is_open() {
                        // A modal captures all input. Messages dismiss on any key;
                        // pickers navigate with j/k and act on Enter.
                        if matches!(app.modal, Modal::Message(_) | Modal::Help) {
                            app.modal = Modal::None;
                        } else if matches!(app.modal, Modal::FilterBuild(_)) {
                            match key.code {
                                KeyCode::Esc => app.modal = Modal::None,
                                KeyCode::Enter => reschedule = save_current_filter(app),
                                KeyCode::Tab | KeyCode::Down => {
                                    if let Modal::FilterBuild(d) = &mut app.modal {
                                        d.move_focus(1);
                                    }
                                }
                                KeyCode::BackTab | KeyCode::Up => {
                                    if let Modal::FilterBuild(d) = &mut app.modal {
                                        d.move_focus(-1);
                                    }
                                }
                                // Arrows jump by section and work anywhere, including
                                // the name field (where h/l are literal text).
                                KeyCode::Right => {
                                    if let Modal::FilterBuild(d) = &mut app.modal {
                                        d.jump_section(1);
                                    }
                                }
                                KeyCode::Left => {
                                    if let Modal::FilterBuild(d) = &mut app.modal {
                                        d.jump_section(-1);
                                    }
                                }
                                KeyCode::Backspace => {
                                    if let Modal::FilterBuild(d) = &mut app.modal
                                        && d.focus == 0
                                    {
                                        d.name.pop();
                                    }
                                }
                                // On the name row every printable key edits the name;
                                // on an option row j/k move, h/l jump sections, and
                                // space toggles.
                                KeyCode::Char(c) => {
                                    if let Modal::FilterBuild(d) = &mut app.modal {
                                        if d.focus == 0 {
                                            d.name.push(c);
                                        } else {
                                            match c {
                                                ' ' => d.toggle_focused(),
                                                'j' => d.move_focus(1),
                                                'k' => d.move_focus(-1),
                                                'l' => d.jump_section(1),
                                                'h' => d.jump_section(-1),
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            match key.code {
                                KeyCode::Char('j') | KeyCode::Down => app.modal_move(1),
                                KeyCode::Char('k') | KeyCode::Up => app.modal_move(-1),
                                KeyCode::Enter => {
                                    if let Some((id, status)) = app.modal_status_pick() {
                                        begin_status_move(app, id, status, &write_tx).await;
                                    } else if matches!(app.modal, Modal::Confirm { .. }) {
                                        confirm_start_work(app, &write_tx).await;
                                    } else if matches!(app.modal, Modal::ConfirmDelete { .. }) {
                                        reschedule = confirm_delete_preset(app);
                                    } else if let Modal::ProjectPick { names, selected } = &app.modal {
                                        let selected = *selected;
                                        if selected >= names.len() {
                                            add_board(terminal, app, &mut poll_handle, &poll_tx, &mut input_rx).await;
                                        } else {
                                            switch_to_project(terminal, app, selected, &mut poll_handle, &poll_tx).await;
                                        }
                                        reschedule = true; // load detail for the new board's selection
                                    } else {
                                        app.modal = Modal::None;
                                    }
                                }
                                KeyCode::Char('y') if matches!(app.modal, Modal::Confirm { .. }) => {
                                    confirm_start_work(app, &write_tx).await;
                                }
                                KeyCode::Char('y') if matches!(app.modal, Modal::ConfirmDelete { .. }) => {
                                    reschedule = confirm_delete_preset(app);
                                }
                                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('n') => {
                                    app.modal = Modal::None;
                                }
                                _ => {}
                            }
                        }
                    } else {
                        match app.input_mode {
                            InputMode::Normal => match key.code {
                                KeyCode::Char('q') => break,
                                KeyCode::Char('j') | KeyCode::Down => reschedule = app.next(),
                                KeyCode::Char('k') | KeyCode::Up => reschedule = app.prev(),
                                KeyCode::Tab => reschedule = app.cycle_preset(1),
                                KeyCode::BackTab => reschedule = app.cycle_preset(-1),
                                KeyCode::Char(c @ '1'..='9') => {
                                    reschedule = app.set_preset(c as usize - '1' as usize);
                                }
                                KeyCode::Char('/') => app.enter_filter(),
                                KeyCode::Char('f') => app.open_filter_builder(),
                                KeyCode::Char('e') => app.open_filter_editor(),
                                KeyCode::Char('d') => begin_delete_preset(app),
                                KeyCode::Char('s') => begin_start_work(app).await,
                                KeyCode::Char('m') => open_status_mover(app).await,
                                KeyCode::Char('p') => open_project_picker(app),
                                KeyCode::Char('o') => open_in_browser(app).await,
                                KeyCode::Char('r') => poll::refresh_now(
                                    app.config.board.owner.clone(),
                                    app.config.board.number,
                                    poll_tx.clone(),
                                ),
                                KeyCode::Char('?') => app.modal = Modal::Help,
                                _ => {}
                            },
                            InputMode::Filter => match key.code {
                                KeyCode::Esc => { app.cancel_filter(); reschedule = true; }
                                KeyCode::Enter => app.confirm_filter(),
                                KeyCode::Backspace => { app.pop_filter(); reschedule = true; }
                                KeyCode::Char(c) => { app.push_filter(c); reschedule = true; }
                                _ => {}
                            },
                        }
                    }
                    if reschedule {
                        schedule_detail(app, &generation, &detail_tx);
                    }
                }
            }
            maybe_detail = detail_rx.recv() => {
                if let Some((g, id, res)) = maybe_detail {
                    // Cache successful fetches even if the selection has moved on.
                    if let Ok(d) = &res {
                        app.detail_cache.insert(id, d.clone());
                    }
                    if g == generation.load(Ordering::SeqCst) {
                        app.detail = match res {
                            Ok(d) => DetailState::Loaded(d),
                            Err(e) => DetailState::Error(e.to_string()),
                        };
                    }
                }
            }
            maybe_write = write_rx.recv() => {
                if let Some((item_id, revert, res)) = maybe_write
                    && let Err(e) = res
                {
                    // Roll back the optimistic move and report.
                    app.set_item_status(&item_id, revert);
                    app.modal = Modal::Message(format!("Status update failed (reverted):\n{e}"));
                }
            }
            maybe_poll = poll_rx.recv() => {
                if let Some((owner, number, items)) = maybe_poll
                    // Ignore snapshots from a board we've since switched away from.
                    && owner == app.config.board.owner
                    && number == app.config.board.number
                    && items != app.items
                {
                    // External change (or `r`): adopt the snapshot, keep selection.
                    let keep = app.selected().map(|i| i.id.clone());
                    app.items = items;
                    app.recompute(keep);
                }
            }
        }
    }
    Ok(())
}

/// Bump the generation, set the pane to Loading (or serve from cache), and spawn
/// a debounced fetch for the current selection. Stale fetches are discarded.
fn schedule_detail(
    app: &mut App,
    generation: &Arc<AtomicU64>,
    detail_tx: &mpsc::UnboundedSender<DetailMsg>,
) {
    let g = generation.fetch_add(1, Ordering::SeqCst) + 1;

    let target = app
        .selected()
        .map(|i| (i.id.clone(), i.number, i.repository.clone()));
    let (id, number, repo) = match target {
        None => {
            app.detail = DetailState::Empty;
            return;
        }
        Some((id, Some(number), Some(repo))) => (id, number, repo),
        Some(_) => {
            app.detail = DetailState::Draft;
            return;
        }
    };

    if let Some(cached) = app.detail_cache.get(&id) {
        app.detail = DetailState::Loaded(cached.clone());
        return;
    }

    // Seed from the on-disk cache before hitting the API on a fresh launch.
    if let Some(c) = cache::load_detail(&repo, number)
        && c.age() < cache::DETAIL_TTL
    {
        app.detail_cache.insert(id.clone(), c.value.clone());
        app.detail = DetailState::Loaded(c.value);
        return;
    }

    app.detail = DetailState::Loading;
    let generation = Arc::clone(generation);
    let tx = detail_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        if generation.load(Ordering::SeqCst) != g {
            return; // superseded during the settle window
        }
        let res = gh::issue::view(&repo, number).await;
        if let Ok(d) = &res {
            cache::save_detail(&repo, number, d);
        }
        let _ = tx.send((g, id, res));
    });
}

/// Validate the start-work preconditions (real issue, linked skill, tmux session,
/// `claude` window, not busy) and open the confirm modal, or a warning modal
/// explaining why we can't proceed.
async fn begin_start_work(app: &mut App) {
    let (item_id, number) = match app.selected() {
        None => return,
        Some(item) => (item.id.clone(), item.number),
    };
    let Some(issue) = number else {
        app.modal = Modal::Message("Draft items have no issue number to start.".into());
        return;
    };
    let Some(skill) = app.config.skill.start.clone() else {
        app.modal = Modal::Message(
            "No start-work skill linked for this project. Add a [projects.skill] start = \"…\" entry to config.".into(),
        );
        return;
    };

    let session = match tmux::current_session().await {
        Ok(Some(s)) => s,
        Ok(None) => {
            app.modal = Modal::Message(
                "Not inside tmux — start-work drives the project's tmux session.".into(),
            );
            return;
        }
        Err(e) => {
            app.modal = Modal::Message(format!("tmux error: {e}"));
            return;
        }
    };

    match tmux::has_claude_window(&session).await {
        Ok(true) => {}
        Ok(false) => {
            app.modal = Modal::Message(format!(
                "Session '{session}' has no 'claude' window to drive."
            ));
            return;
        }
        Err(e) => {
            app.modal = Modal::Message(format!("tmux error: {e}"));
            return;
        }
    }

    match tmux::is_busy(&session).await {
        Ok(true) => {
            app.modal = Modal::Message(format!(
                "Claude is busy in '{session}'. Wait for it to finish before starting a new ticket."
            ));
            return;
        }
        Ok(false) => {}
        Err(e) => {
            app.modal = Modal::Message(format!("tmux error: {e}"));
            return;
        }
    }

    app.modal = Modal::Confirm {
        item_id,
        issue,
        skill,
        session,
    };
}

/// Drive the confirmed start: `/clear` the claude pane, invoke the skill, then
/// best-effort auto-flip the card to In progress.
async fn confirm_start_work(app: &mut App, write_tx: &mpsc::UnboundedSender<WriteMsg>) {
    let Modal::Confirm {
        item_id,
        issue,
        skill,
        session,
    } = &app.modal
    else {
        return;
    };
    let (item_id, issue, skill, session) =
        (item_id.clone(), *issue, skill.clone(), session.clone());
    app.modal = match tmux::start_work(&session, &skill, issue).await {
        Ok(()) => {
            let flipped = try_auto_flip(app, &item_id, write_tx).await;
            let note = if flipped {
                " · moved to In progress"
            } else {
                ""
            };
            Modal::Message(format!(
                "Started #{issue} with '{skill}' in {session}:claude.{note}"
            ))
        }
        Err(e) => Modal::Message(format!("Start-work failed: {e}")),
    };
}

/// Open the selected ticket in the browser via `gh issue view --web`.
async fn open_in_browser(app: &mut App) {
    let Some((number, repo)) = app.selected().map(|i| (i.number, i.repository.clone())) else {
        return; // no selection
    };
    match (number, repo) {
        (Some(n), Some(repo)) => {
            if let Err(e) = gh::issue::open_web(&repo, n).await {
                app.modal = Modal::Message(format!("Couldn't open in browser:\n{e}"));
            }
        }
        _ => app.modal = Modal::Message("Draft item — no issue to open in the browser.".into()),
    }
}

/// Commit the open filter builder: apply the drafted preset in-memory (making it
/// active) and persist it to `config.toml`. A blank name is a no-op that leaves
/// the builder open. Returns whether the view changed (caller reschedules detail).
fn save_current_filter(app: &mut App) -> bool {
    let (preset, replacing) = match &app.modal {
        Modal::FilterBuild(draft) => (draft.to_preset(), draft.original.clone()),
        _ => (None, None),
    };
    let Some(preset) = preset else {
        // Empty name — keep the builder open so the user can fill it in.
        return false;
    };

    app.save_preset(preset.clone(), replacing.clone());
    app.modal = Modal::None;

    // In-memory state is already updated; surface a disk-write failure without
    // losing the session's filter.
    if let Err(e) = persist_preset(&app.config.name, &preset, replacing.as_deref()) {
        app.modal = Modal::Message(format!("Filter is active for this session, but:\n{e}"));
    }
    true
}

/// Write `preset` into the on-disk config under `project_name`. `replacing` is
/// the pre-edit name: on a rename the old entry is dropped first. Otherwise a
/// same-named preset is overwritten in place, else appended.
fn persist_preset(
    project_name: &str,
    preset: &config::schema::Preset,
    replacing: Option<&str>,
) -> anyhow::Result<()> {
    use anyhow::Context;
    let mut cfg = Config::load()?;
    let project = cfg
        .projects
        .iter_mut()
        .find(|p| p.name == project_name)
        .context("this board isn't in your config yet — the filter can't be saved to disk")?;
    if let Some(orig) = replacing
        && !orig.eq_ignore_ascii_case(&preset.name)
    {
        project
            .presets
            .retain(|p| !p.name.eq_ignore_ascii_case(orig));
    }
    match project
        .presets
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(&preset.name))
    {
        Some(i) => project.presets[i] = preset.clone(),
        None => project.presets.push(preset.clone()),
    }
    cfg.save()
}

/// Open the delete-confirm modal for the active preset, or refuse (with a note)
/// when it's the only one left — the preset list must stay non-empty.
fn begin_delete_preset(app: &mut App) {
    if app.preset_count() <= 1 {
        app.modal =
            Modal::Message("Can't delete the only filter — a board needs at least one.".into());
        return;
    }
    let index = app.active_preset;
    let name = app.preset_name(index).to_string();
    app.modal = Modal::ConfirmDelete { index, name };
}

/// Delete the preset named by the open `ConfirmDelete` modal, in memory and on
/// disk, then rebuild the view. Returns whether the view changed.
fn confirm_delete_preset(app: &mut App) -> bool {
    let (index, name) = match &app.modal {
        Modal::ConfirmDelete { index, name } => (*index, name.clone()),
        _ => return false,
    };
    if !app.delete_preset(index) {
        app.modal = Modal::None;
        return false;
    }
    app.modal = Modal::None;
    if let Err(e) = persist_delete(&app.config.name, &name) {
        app.modal = Modal::Message(format!("Filter removed for this session, but:\n{e}"));
    }
    true
}

/// Remove the preset named `preset_name` from the on-disk config under `project_name`.
fn persist_delete(project_name: &str, preset_name: &str) -> anyhow::Result<()> {
    use anyhow::Context;
    let mut cfg = Config::load()?;
    let project = cfg
        .projects
        .iter_mut()
        .find(|p| p.name == project_name)
        .context("this board isn't in your config yet — nothing to remove from disk")?;
    project
        .presets
        .retain(|p| !p.name.eq_ignore_ascii_case(preset_name));
    cfg.save()
}

/// Open the project switcher, listing every configured project (freshly reloaded
/// from disk, so wizard-added boards appear) plus an "Add a board…" entry.
fn open_project_picker(app: &mut App) {
    let cfg = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            app.modal = Modal::Message(format!("Couldn't read config:\n{e}"));
            return;
        }
    };
    let names: Vec<String> = cfg.projects.iter().map(|p| p.name.clone()).collect();
    let selected = names
        .iter()
        .position(|n| *n == app.config.name)
        .unwrap_or(0);
    app.modal = Modal::ProjectPick { names, selected };
}

/// Switch the active board to the configured project at `index` (positional, so
/// duplicate project names stay unambiguous): reload config, fetch its board,
/// swap App state, and re-target the poller. On failure the current board is left
/// untouched and a message is shown.
async fn switch_to_project(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    index: usize,
    poll_handle: &mut tokio::task::JoinHandle<()>,
    poll_tx: &mpsc::UnboundedSender<poll::Snapshot>,
) {
    app.modal = Modal::None;
    let cfg = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            app.modal = Modal::Message(format!("Couldn't read config:\n{e}"));
            return;
        }
    };
    let Some(project) = cfg.projects.get(index).cloned() else {
        app.modal = Modal::Message("That project is no longer in the config.".into());
        return;
    };
    load_board_into(terminal, app, project, poll_handle, poll_tx).await;
}

/// Run the first-run wizard live (against the current repo) to register a new
/// board, then switch to it. Cancelling returns to the current board silently.
async fn add_board(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    poll_handle: &mut tokio::task::JoinHandle<()>,
    poll_tx: &mpsc::UnboundedSender<poll::Snapshot>,
    input_rx: &mut mpsc::UnboundedReceiver<Event>,
) {
    app.modal = Modal::None;
    let cfg = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            app.modal = Modal::Message(format!("Couldn't read config:\n{e}"));
            return;
        }
    };
    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(e) => {
            app.modal = Modal::Message(format!("Couldn't read the working directory:\n{e}"));
            return;
        }
    };
    let repo = resolver::repo_at(&cwd);
    // The wizard takes over the whole screen and writes the new project to disk.
    // Any error (cancel / no remote — it shows its own message) just returns us
    // to the current board.
    if let Ok(project) = config::wizard::run(
        terminal,
        repo,
        &cwd,
        cfg,
        false,
        config::wizard::Input::Channel(input_rx),
    )
    .await
    {
        load_board_into(terminal, app, project, poll_handle, poll_tx).await;
    }
}

/// Fetch `project`'s board and adopt it into `app`, re-targeting the poller.
async fn load_board_into(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    project: config::schema::ProjectConfig,
    poll_handle: &mut tokio::task::JoinHandle<()>,
    poll_tx: &mpsc::UnboundedSender<poll::Snapshot>,
) {
    let _ = terminal.draw(|f| {
        ui::render(f, app);
        ui::modal::render_notice(
            f,
            &format!("Loading {} #{}…", project.name, project.board.number),
        );
    });
    match load_board(&project.board.owner, project.board.number).await {
        Ok(items) => {
            let (owner, number) = (project.board.owner.clone(), project.board.number);
            app.switch_board(project, items);
            poll_handle.abort();
            *poll_handle = poll::spawn(owner, number, poll_tx.clone());
        }
        Err(e) => app.modal = Modal::Message(format!("Couldn't load board:\n{e}")),
    }
}

/// Open the status mover for the selected card, fetching the board's Status
/// field on first use.
async fn open_status_mover(app: &mut App) {
    let Some(item_id) = app.selected().map(|i| i.id.clone()) else {
        return;
    };
    if !ensure_status_field(app).await {
        return; // ensure_status_field set an error modal
    }
    let options = app.status_field.as_ref().unwrap().names();
    let selected = app
        .item_status(&item_id)
        .and_then(|cur| options.iter().position(|o| o.eq_ignore_ascii_case(&cur)))
        .unwrap_or(0);
    app.modal = Modal::StatusMove {
        item_id,
        options,
        selected,
    };
}

/// Apply a manual status move: check scope, optimistically update, and fire the
/// write in the background.
async fn begin_status_move(
    app: &mut App,
    item_id: String,
    new_status: String,
    write_tx: &mpsc::UnboundedSender<WriteMsg>,
) {
    if !ensure_scope(app).await {
        app.modal = Modal::Message(SCOPE_HINT.into());
        return;
    }
    let Some(field) = app.status_field.clone() else {
        app.modal = Modal::None;
        return;
    };
    let Some(option_id) = field.option_id(&new_status).map(str::to_string) else {
        app.modal = Modal::Message(format!("Unknown status '{new_status}'."));
        return;
    };

    let old = app.set_item_status(&item_id, Some(new_status));
    app.modal = Modal::None;
    spawn_status_write(field, item_id, option_id, old, write_tx);
}

/// After a successful start, move the card to In progress if it isn't already —
/// silent best-effort (skipped without the write scope). Returns whether it fired.
async fn try_auto_flip(
    app: &mut App,
    item_id: &str,
    write_tx: &mpsc::UnboundedSender<WriteMsg>,
) -> bool {
    const TARGET: &str = "In progress";
    if app
        .item_status(item_id)
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case(TARGET))
    {
        return false;
    }
    if !ensure_scope(app).await {
        return false;
    }
    // Fetch the Status field silently (don't nag with an error modal here).
    if app.status_field.is_none() {
        match gh::write::status_field(&app.config.board.owner, app.config.board.number).await {
            Ok(sf) => app.status_field = Some(sf),
            Err(_) => return false,
        }
    }
    let field = app.status_field.clone().unwrap();
    let Some(option_id) = field.option_id(TARGET).map(str::to_string) else {
        return false;
    };
    let old = app.set_item_status(item_id, Some(TARGET.to_string()));
    spawn_status_write(field, item_id.to_string(), option_id, old, write_tx);
    true
}

/// Spawn the background `gh` write and report its outcome on the write channel.
fn spawn_status_write(
    field: gh::write::StatusField,
    item_id: String,
    option_id: String,
    revert: Option<String>,
    write_tx: &mpsc::UnboundedSender<WriteMsg>,
) {
    let tx = write_tx.clone();
    tokio::spawn(async move {
        let res = gh::write::set_status(&field, &item_id, &option_id).await;
        let _ = tx.send((item_id, revert, res));
    });
}

/// Cache and return whether the token has the `project` write scope.
async fn ensure_scope(app: &mut App) -> bool {
    if app.project_scope.is_none() {
        app.project_scope = Some(gh::scope::has_project_scope().await.unwrap_or(false));
    }
    app.project_scope == Some(true)
}

/// Cache the board's Status field; on failure set an error modal and return false.
async fn ensure_status_field(app: &mut App) -> bool {
    if app.status_field.is_some() {
        return true;
    }
    match gh::write::status_field(&app.config.board.owner, app.config.board.number).await {
        Ok(sf) => {
            app.status_field = Some(sf);
            true
        }
        Err(e) => {
            app.modal = Modal::Message(format!("Couldn't read board Status field:\n{e}"));
            false
        }
    }
}
