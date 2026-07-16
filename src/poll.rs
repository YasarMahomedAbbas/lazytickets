//! Background board refresh. A tokio timer re-fetches the board off the UI
//! thread every ~45s; the main loop diffs the result and repaints only on a real
//! change. Rate limits are a non-issue (one GraphQL call per poll).

use crate::gh;
use crate::model::Item;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// A board snapshot tagged with the board it came from, so the UI can ignore
/// deliveries from a previously-active board after a project switch.
pub type Snapshot = (String, u32, Vec<Item>);

/// Poll interval — within the 30–60s the design note settled on.
const INTERVAL: Duration = Duration::from_secs(45);

/// Spawn the background poller for `(owner, number)`. It sends a fresh board
/// snapshot on every tick; failed fetches are skipped silently (the next tick
/// retries). The handle lets the caller `.abort()` it when switching boards.
pub fn spawn(owner: String, number: u32, tx: mpsc::UnboundedSender<Snapshot>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERVAL);
        ticker.tick().await; // swallow the immediate first tick (already loaded)
        loop {
            ticker.tick().await;
            if let Ok(items) = gh::project::item_list(&owner, number).await
                && tx.send((owner.clone(), number, items)).is_err()
            {
                break; // UI gone
            }
        }
    })
}

/// Fetch the board once, now, and send it on `tx` (drives the `r` refresh key).
pub fn refresh_now(owner: String, number: u32, tx: mpsc::UnboundedSender<Snapshot>) {
    tokio::spawn(async move {
        if let Ok(items) = gh::project::item_list(&owner, number).await {
            let _ = tx.send((owner, number, items));
        }
    });
}
