//! Background board refresh. Board state changes infrequently, so we poll on a
//! long cadence — Projects v2 is GraphQL (point-expensive, no conditional
//! requests), and a snapshot matching the current board is a no-op in the UI.
//! Every successful fetch also updates the on-disk cache, so a relaunch renders
//! instantly without an API call. On a rate-limit rejection we back off
//! exponentially rather than retry on the next tick: continued requests during a
//! secondary limit only extend its window. The `r` key forces an immediate fetch.

use crate::cache;
use crate::gh;
use crate::model::Item;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// A board snapshot tagged with the board it came from, so the UI can ignore
/// deliveries from a previously-active board after a project switch.
pub type Snapshot = (String, u32, Vec<Item>);

/// Base poll interval. Tickets rarely change, so this stays long to keep us well
/// clear of the Projects v2 API budget; `r` covers the "I want it now" case.
const INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Upper bound on rate-limit backoff, so the poller eventually recovers.
const MAX_BACKOFF: Duration = Duration::from_secs(2 * 60 * 60);

/// Spawn the background poller for `(owner, number)`. It sends a fresh board
/// snapshot on every successful tick and caches it to disk; rate-limit failures
/// trigger exponential backoff, other failures retry on the next normal tick. The
/// handle lets the caller `.abort()` it when switching boards.
pub fn spawn(owner: String, number: u32, tx: mpsc::UnboundedSender<Snapshot>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff: Option<Duration> = None;
        loop {
            tokio::time::sleep(backoff.unwrap_or(INTERVAL)).await;
            match gh::project::item_list(&owner, number).await {
                Ok(items) => {
                    backoff = None;
                    cache::save_board(&owner, number, &items);
                    if tx.send((owner.clone(), number, items)).is_err() {
                        break; // UI gone
                    }
                }
                Err(e) if gh::is_rate_limit(&e) => {
                    // Lengthen the wait each consecutive limit, capped.
                    let next = backoff.map_or(INTERVAL * 2, |b| b * 2).min(MAX_BACKOFF);
                    backoff = Some(next);
                }
                Err(_) => backoff = None, // transient blip: retry next interval
            }
        }
    })
}

/// Fetch the board once, now, cache it, and send it on `tx` (drives the `r`
/// refresh key).
pub fn refresh_now(owner: String, number: u32, tx: mpsc::UnboundedSender<Snapshot>) {
    tokio::spawn(async move {
        if let Ok(items) = gh::project::item_list(&owner, number).await {
            cache::save_board(&owner, number, &items);
            let _ = tx.send((owner, number, items));
        }
    });
}
