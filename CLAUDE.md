# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A lazygit-style Rust TUI (`ratatui` + `crossterm` + `tokio`) for GitHub Projects v2
boards, run inside tmux. It reads a board, shows issue detail, filters via presets +
fuzzy search, writes card status, and — the headline feature — drives the Claude Code
session in the project's `claude` tmux window to start work on a ticket.

Note: the crate and on-disk config are named **`lazytickets`** (repo directory is
`lazyissues`). `PLAN.md` is the authoritative design/build-order doc; v1 (milestones
M0–M7) is complete.

## Commands

```bash
cargo run            # launch the TUI (resolves cwd's git repo → its board)
cargo build          # release: cargo build --release → target/release/lazytickets
cargo test           # all unit tests (fast, no network — see Testing below)
cargo test <name>    # single test by substring, e.g. cargo test normalizes_remote
cargo test optimistic_status_update_and_revert
```

Edition 2024, pinned toolchain (no rustup). Tests are pure and hermetic — they never
hit the network or spawn `gh`.

## Architecture

Two hard rules structure the whole codebase:

1. **`gh/mod.rs` is the only module that spawns `gh` subprocesses.** Everything else
   works on typed structs from `model.rs`. `gh::run(&[args])` is the single choke point;
   `gh/{project,issue,scope,write}.rs` build args and parse JSON into domain types.
2. **`tmux.rs` is the only module that spawns `tmux`.** All session detection, the
   busy-guard, and `send-keys` start-work driving live there.

### Event loop (`main.rs::run`)

A single `tokio::select!` loop multiplexes four channels; `app.rs::App` holds all state
and `ui/` only renders it (`ui::render(frame, app)` is pure over `&App`).

- **Input** — a blocking `event::read()` thread feeds an async channel, so key handling
  never stalls the runtime while detail fetches are in flight.
- **Detail fetches** — debounced (~150ms settle) and **generation-guarded**: each
  selection bumps an `AtomicU64`; a fetch that finishes after the selection moved on is
  discarded (but still cached). See `schedule_detail`. Results cached in `detail_cache`.
- **Status writes** — optimistic: `app.set_item_status` mutates in-memory state and
  re-sorts immediately, the `gh` write fires in the background, and a failure reverts and
  shows a Message modal.
- **Poll** — `poll.rs` re-fetches the board every ~45s off the UI thread; the loop adopts
  a snapshot only when it differs (`Item: PartialEq`), preserving selection. The `r`
  force-refresh shares this same channel (`poll::refresh_now`).

### Resolution & config (`config/`)

On launch, before touching the terminal, `resolver::resolve(cwd)` maps the current repo to
a project, trying in order: (1) path-keyed override, (2) git-remote `origin` normalised to
`owner/repo` matched against a project's `repos`, (3) unknown → the in-TUI first-run
`wizard`. Config lives at `~/.config/lazytickets/config.toml` (XDG via `directories`).
`schema.rs` holds both the serde types **and** the pure filter/sort logic (`Filter::matches`,
`ProjectConfig::keeps`, `status_rank`) — the filtering engine, unit-tested there.

### Filtering model

`App::recompute` rebuilds `visible` (indices into `items`) = active preset filter →
`exclude_statuses` (unless the preset names that status) → fuzzy query, then sorts by
`status_order` rank. Within a filter field matches are OR, across fields AND.

### Start-work flow (M5, the headline feature)

`s` → `begin_start_work` validates preconditions (real issue not draft, a linked skill in
config, inside tmux, session has a `claude` window, not busy) → Confirm modal → on `y`,
`tmux::start_work` sends `/clear` then a literal `Use the <skill> skill for issue #<n>`
line, then best-effort auto-flips the card to *In progress*. The **busy-guard** reads the
`@claude_busy` tmux option with a pane-ownership check (copied from `sesh-list-bells`) so we
never `/clear` Claude mid-task.

## Conventions

- `anyhow` in app code; `thiserror` reserved for typed errors in lower layers.
- Modules stay small; this is deliberately a first Rust project — no async trait objects,
  no generic source abstraction (deferred to the v2 pluggable-source work in `PLAN.md §7`).
- `#[allow(dead_code)]` markers on `Item` fields are intentional (consumed by later
  milestones); keep them.
- Keybindings are all plain letters to avoid clashing with tmux `C-f` and ghostty
  `Ctrl+Shift+*`. Full list in `PLAN.md §3` / the `?` help overlay.

## Testing

Unit tests are colocated (`#[cfg(test)] mod tests`) and cover the pure logic: remote-URL
normalisation, preset filter/exclude/sort, config TOML round-trip, optimistic-update revert.
The `gh` JSON layer is tested against captured fixtures in `tests/fixtures/` — no network.
tmux driving and live board writes are exercised by hand (this machine's token may lack the
`project` scope; a live write mutates a real board).
