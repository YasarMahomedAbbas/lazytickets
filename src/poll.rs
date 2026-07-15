//! Background board refresh. A tokio timer re-fetches the board off the UI
//! thread every ~45s; the main loop diffs the result and repaints only on a real
//! change. Rate limits are a non-issue (one GraphQL call per poll).

use crate::gh;
use crate::model::Item;
use std::time::Duration;
use tokio::sync::mpsc;

/// Poll interval — within the 30–60s the design note settled on.
const INTERVAL: Duration = Duration::from_secs(45);

/// Spawn the background poller. It sends a fresh board snapshot on every tick;
/// failed fetches are skipped silently (the next tick retries).
pub fn spawn(owner: String, number: u32, tx: mpsc::UnboundedSender<Vec<Item>>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERVAL);
        ticker.tick().await; // swallow the immediate first tick (already loaded)
        loop {
            ticker.tick().await;
            if let Ok(items) = gh::project::item_list(&owner, number).await
                && tx.send(items).is_err()
            {
                break; // UI gone
            }
        }
    });
}

/// Fetch the board once, now, and send it on `tx` (drives the `r` refresh key).
pub fn refresh_now(owner: String, number: u32, tx: mpsc::UnboundedSender<Vec<Item>>) {
    tokio::spawn(async move {
        if let Ok(items) = gh::project::item_list(&owner, number).await {
            let _ = tx.send(items);
        }
    });
}
