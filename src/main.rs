mod app;
mod config;
mod gh;
mod model;
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
                        // A modal captures all input: confirm start-work, else dismiss.
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Enter
                                if matches!(app.modal, Modal::Confirm { .. }) =>
                            {
                                confirm_start_work(app).await;
                            }
                            _ => app.modal = Modal::None,
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
    let number = match app.selected() {
        None => return,
        Some(item) => item.number,
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

    app.modal = Modal::Confirm { issue, skill, session };
}

/// Drive the confirmed start: `/clear` the claude pane, then invoke the skill.
async fn confirm_start_work(app: &mut App) {
    let Modal::Confirm { issue, skill, session } = &app.modal else {
        return;
    };
    let (issue, skill, session) = (*issue, skill.clone(), session.clone());
    app.modal = match tmux::start_work(&session, &skill, issue).await {
        Ok(()) => Modal::Message(format!("Started #{issue} with '{skill}' in {session}:claude.")),
        Err(e) => Modal::Message(format!("Start-work failed: {e}")),
    };
}
