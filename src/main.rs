mod app;
mod config;
mod gh;
mod model;
mod poll;
mod tmux;
mod ui;

use app::{App, DetailState, InputMode, Modal};
use config::Config;
use config::resolver::{self, Resolution};
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
const SCOPE_HINT: &str =
    "Status writes need the 'project' scope. Quit and run:\n  gh auth refresh -s project\nthen relaunch.";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Resolve the current repo → its board (config), before touching the terminal.
    let config = Config::load()?;
    let cwd = std::env::current_dir()?;
    let resolution = resolver::resolve(&config, &cwd);

    let mut terminal = ratatui::init();

    // Known repo → open directly; unknown → the first-run wizard writes a config.
    let cfg = match resolution {
        Resolution::Project(p) => *p,
        Resolution::Unknown { repo } => match config::wizard::run(&mut terminal, repo, &cwd, config).await {
            Ok(p) => p,
            Err(e) => {
                ratatui::restore();
                return Err(e);
            }
        },
    };

    let result = match gh::project::item_list(&cfg.board.owner, cfg.board.number).await {
        Ok(items) => {
            let mut app = App::new(items, cfg);
            run(&mut terminal, &mut app).await
        }
        Err(e) => Err(e),
    };
    ratatui::restore();
    result
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
    let (poll_tx, mut poll_rx) = mpsc::unbounded_channel::<Vec<model::Item>>();
    poll::spawn(app.config.board.owner.clone(), app.config.board.number, poll_tx.clone());

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
                        } else {
                            match key.code {
                                KeyCode::Char('j') | KeyCode::Down => app.modal_move(1),
                                KeyCode::Char('k') | KeyCode::Up => app.modal_move(-1),
                                KeyCode::Enter => {
                                    if let Some((id, status)) = app.modal_status_pick() {
                                        begin_status_move(app, id, status, &write_tx).await;
                                    } else if matches!(app.modal, Modal::Confirm { .. }) {
                                        confirm_start_work(app, &write_tx).await;
                                    } else {
                                        app.modal = Modal::None;
                                    }
                                }
                                KeyCode::Char('y') if matches!(app.modal, Modal::Confirm { .. }) => {
                                    confirm_start_work(app, &write_tx).await;
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
                                KeyCode::Char('s') => begin_start_work(app).await,
                                KeyCode::Char('m') => open_status_mover(app).await,
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
                if let Some(items) = maybe_poll
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
fn schedule_detail(app: &mut App, generation: &Arc<AtomicU64>, detail_tx: &mpsc::UnboundedSender<DetailMsg>) {
    let g = generation.fetch_add(1, Ordering::SeqCst) + 1;

    let target = app.selected().map(|i| (i.id.clone(), i.number, i.repository.clone()));
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

    app.detail = DetailState::Loading;
    let generation = Arc::clone(generation);
    let tx = detail_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        if generation.load(Ordering::SeqCst) != g {
            return; // superseded during the settle window
        }
        let res = gh::issue::view(&repo, number).await;
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
            app.modal = Modal::Message("Not inside tmux — start-work drives the project's tmux session.".into());
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
            app.modal = Modal::Message(format!("Session '{session}' has no 'claude' window to drive."));
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

    app.modal = Modal::Confirm { item_id, issue, skill, session };
}

/// Drive the confirmed start: `/clear` the claude pane, invoke the skill, then
/// best-effort auto-flip the card to In progress.
async fn confirm_start_work(app: &mut App, write_tx: &mpsc::UnboundedSender<WriteMsg>) {
    let Modal::Confirm { item_id, issue, skill, session } = &app.modal else {
        return;
    };
    let (item_id, issue, skill, session) = (item_id.clone(), *issue, skill.clone(), session.clone());
    app.modal = match tmux::start_work(&session, &skill, issue).await {
        Ok(()) => {
            let flipped = try_auto_flip(app, &item_id, write_tx).await;
            let note = if flipped { " · moved to In progress" } else { "" };
            Modal::Message(format!("Started #{issue} with '{skill}' in {session}:claude.{note}"))
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
    app.modal = Modal::StatusMove { item_id, options, selected };
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
async fn try_auto_flip(app: &mut App, item_id: &str, write_tx: &mpsc::UnboundedSender<WriteMsg>) -> bool {
    const TARGET: &str = "In progress";
    if app.item_status(item_id).as_deref().is_some_and(|s| s.eq_ignore_ascii_case(TARGET)) {
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
