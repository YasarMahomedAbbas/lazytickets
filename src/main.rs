mod app;
mod gh;
mod model;
mod ui;

use app::{App, DetailState};
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
    // M1: resolver hardcoded to travel-smart board #6 (git-remote resolution + wizard land in M4).
    let items = gh::project::item_list("WhiteWolfStudio", 6).await?;
    let mut app = App::new(items, "travel-smart #6".to_string());

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app).await;
    ratatui::restore();
    result
}

async fn run(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    // Input: a blocking reader thread feeding an async channel, so key handling
    // never blocks the tokio runtime (which is busy with detail fetches).
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Event>();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if input_tx.send(ev).is_err() {
                break; // UI gone
            }
        }
    });

    // Detail fetches: debounced and generation-guarded so fast scrolling doesn't
    // fire a `gh` call per row and stale results are dropped.
    let (detail_tx, mut detail_rx) = mpsc::unbounded_channel::<DetailMsg>();
    let generation = Arc::new(AtomicU64::new(0));

    // Kick off the fetch for the initially-selected item.
    schedule_detail(app, &generation, &detail_tx);

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        tokio::select! {
            maybe_ev = input_rx.recv() => {
                let Some(ev) = maybe_ev else { break }; // input thread ended
                if let Event::Key(key) = ev
                    && key.kind == KeyEventKind::Press
                {
                    let moved = match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('j') | KeyCode::Down => app.next(),
                        KeyCode::Char('k') | KeyCode::Up => app.prev(),
                        _ => false,
                    };
                    if moved {
                        schedule_detail(app, &generation, &detail_tx);
                    }
                }
            }
            maybe_detail = detail_rx.recv() => {
                if let Some((g, id, res)) = maybe_detail {
                    // Cache successful fetches even if the selection has moved on —
                    // the work is done, so a later revisit should still be instant.
                    if let Ok(d) = &res {
                        app.detail_cache.insert(id, d.clone());
                    }
                    // But only paint it if it's still the current selection.
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

/// Bump the generation, set the pane to Loading, and spawn a debounced fetch for
/// the current selection. A fetch whose generation is stale by the time it wakes
/// (or completes) is discarded.
fn schedule_detail(app: &mut App, generation: &Arc<AtomicU64>, detail_tx: &mpsc::UnboundedSender<DetailMsg>) {
    let g = generation.fetch_add(1, Ordering::SeqCst) + 1;

    // Extract what we need, dropping the borrow before mutating app.detail.
    let target = app.selected().map(|i| (i.id.clone(), i.number, i.repository.clone()));
    let (id, number, repo) = match target {
        None => {
            app.detail = DetailState::Empty;
            return;
        }
        Some((id, Some(number), Some(repo))) => (id, number, repo),
        Some(_) => {
            app.detail = DetailState::Draft; // draft item: nothing to fetch
            return;
        }
    };

    // Cache hit: paint instantly, no fetch, no "Loading…".
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
            return; // superseded during the settle window — skip the call entirely
        }
        let res = gh::issue::view(&repo, number).await;
        let _ = tx.send((g, id, res));
    });
}
