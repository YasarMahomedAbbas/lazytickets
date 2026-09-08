//! Disk cache for board snapshots and issue details, under the XDG cache dir.
//!
//! Projects v2 is served over GraphQL, which has no conditional-request (ETag)
//! support — the only way to cut API traffic is to fetch less often and reuse
//! results across restarts. This persists the last board snapshot per
//! `(owner, number)` and each fetched issue detail, every entry stamped with its
//! fetch time so callers can decide when a refetch is warranted. All writes are
//! best-effort: a cache miss or I/O error just means we fall back to the network.

use crate::gh::issue::IssueDetail;
use crate::model::Item;
use directories::ProjectDirs;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a cached board snapshot is served without hitting the API. Matches
/// the poll cadence: within this window a relaunch is a pure cache read.
pub const BOARD_TTL: Duration = Duration::from_secs(30 * 60);

/// How long a cached issue detail is reused across restarts before refetching.
pub const DETAIL_TTL: Duration = Duration::from_secs(30 * 60);

/// A cached value tagged with when it was fetched (unix seconds).
#[derive(Serialize, serde::Deserialize)]
pub struct Cached<T> {
    fetched_at: u64,
    pub value: T,
}

impl<T> Cached<T> {
    /// Time elapsed since this entry was fetched.
    pub fn age(&self) -> Duration {
        Duration::from_secs(now_unix().saturating_sub(self.fetched_at))
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `~/.cache/lazytickets` (XDG on Linux). `None` if no cache dir resolves.
fn cache_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "lazytickets").map(|d| d.cache_dir().to_path_buf())
}

fn board_path(owner: &str, number: u32) -> Option<PathBuf> {
    Some(
        cache_dir()?
            .join("boards")
            .join(format!("{owner}-{number}.json")),
    )
}

fn detail_path(repo: &str, number: u64) -> Option<PathBuf> {
    // `repo` is `owner/name`; keep it filesystem-safe.
    let safe = repo.replace('/', "__");
    Some(
        cache_dir()?
            .join("details")
            .join(format!("{safe}-{number}.json")),
    )
}

fn read<T: DeserializeOwned>(path: &Path) -> Option<Cached<T>> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write<T: Serialize + ?Sized>(path: &Path, value: &T) {
    // A borrowing twin of `Cached` so we can serialise without cloning `value`.
    #[derive(Serialize)]
    struct Entry<'a, T: ?Sized> {
        fetched_at: u64,
        value: &'a T,
    }
    let entry = Entry {
        fetched_at: now_unix(),
        value,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(&entry) {
        let _ = std::fs::write(path, text);
    }
}

/// A cached board snapshot. `parents` records whether the sub-issue tree was
/// fetched alongside the cards: without it every `Item::parent` is `None`, which
/// is indistinguishable from a board that simply has no sub-issues — so we
/// record it rather than guess, and a view that groups by parent refetches. An
/// entry written before this field existed fails to deserialise (it was a bare
/// array) and reads as a plain cache miss.
#[derive(serde::Deserialize)]
pub struct BoardSnapshot {
    #[serde(default)]
    pub parents: bool,
    pub items: Vec<Item>,
}

pub fn load_board(owner: &str, number: u32) -> Option<Cached<BoardSnapshot>> {
    read(&board_path(owner, number)?)
}

pub fn save_board(owner: &str, number: u32, items: &[Item], parents: bool) {
    // A borrowing twin of `BoardSnapshot`, so the items aren't cloned to write.
    #[derive(Serialize)]
    struct Out<'a> {
        parents: bool,
        items: &'a [Item],
    }
    if let Some(path) = board_path(owner, number) {
        write(&path, &Out { parents, items });
    }
}

/// Write `text` to the scratch Markdown file for `repo#number` (the document
/// `v` opens in the editor) and return its path. `None` if no cache dir resolves
/// or the write fails.
pub fn write_view(repo: &str, number: u64, text: &str) -> Option<PathBuf> {
    let path = cache_dir()?
        .join("view")
        .join(format!("{}-{number}.md", repo.replace('/', "-")));
    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::write(&path, text).ok()?;
    Some(path)
}

pub fn load_detail(repo: &str, number: u64) -> Option<Cached<IssueDetail>> {
    read(&detail_path(repo, number)?)
}

pub fn save_detail(repo: &str, number: u64, detail: &IssueDetail) {
    if let Some(path) = detail_path(repo, number) {
        write(&path, detail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_gates_on_age() {
        // A just-now stamp reads as fresh; an epoch-old one as stale — this is the
        // check that decides launch-from-cache vs. refetch.
        let json = format!("{{\"fetched_at\":{},\"value\":7}}", now_unix());
        let fresh: Cached<u32> = serde_json::from_str(&json).unwrap();
        assert_eq!(fresh.value, 7);
        assert!(fresh.age() < BOARD_TTL);

        let stale: Cached<u32> = serde_json::from_str("{\"fetched_at\":0,\"value\":7}").unwrap();
        assert!(stale.age() > BOARD_TTL);
    }

    #[test]
    #[ignore] // touches the real XDG cache dir; run explicitly to verify the fs layer
    fn board_round_trips_through_disk() {
        use crate::model::Item;
        let item = Item {
            id: "PVTI_verify".into(),
            number: Some(999),
            title: "ROUND-TRIP-PROOF".into(),
            repository: Some("acme/widgets".into()),
            status: Some("Todo".into()),
            labels: vec!["bug".into()],
            assignees: vec!["me".into()],
            url: None,
            parent: Some(crate::model::ParentRef {
                repository: "acme/widgets".into(),
                number: 998,
            }),
        };
        // Save, then read back through the real path computation + serde.
        save_board("verify-owner", 7, std::slice::from_ref(&item), true);
        let back = load_board("verify-owner", 7).expect("cache file should exist");
        assert_eq!(back.value.items.len(), 1);
        assert_eq!(back.value.items[0].title, "ROUND-TRIP-PROOF");
        // The sub-issue tree survives the round trip, flag and all — a snapshot
        // that lost it would render an `include_parents` view flat.
        assert!(back.value.parents);
        assert_eq!(back.value.items[0].parent.as_ref().unwrap().number, 998);
        assert!(
            back.age() < BOARD_TTL,
            "a just-written entry must read as fresh"
        );
        // Clean up so we don't leave a fixture in the user's cache dir.
        let _ = std::fs::remove_file(board_path("verify-owner", 7).unwrap());
    }

    #[test]
    fn detail_path_is_filesystem_safe() {
        // `owner/name` must not introduce a directory separator into the filename.
        let p = detail_path("WhiteWolfStudio/travel-smart", 226).unwrap();
        assert_eq!(
            p.file_name().unwrap(),
            "WhiteWolfStudio__travel-smart-226.json"
        );
    }
}
