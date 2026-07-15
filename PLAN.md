# lazytickets — Implementation Plan

A lazygit-style Rust TUI for GitHub issues/tickets, run inside tmux, that drives the
Claude Code session in the project's `claude` window to start work on a ticket.

This is the build-order companion to the design note (`vault:25-ideas/lazytickets/about.md`).
All v1 scope decisions are settled there; this document is *how* and *in what order*.

---

## 0. Ground rules

- **Language/stack:** Rust edition 2024 (`rustc`/`cargo` 1.96, no rustup), `ratatui` + `crossterm`,
  `tokio` for async, shelling out to `gh` (never reimplement the GitHub API).
- **First Rust project** — keep modules small, lean on the compiler, avoid premature abstraction.
  No async trait objects or generic source abstraction in v1 (that's the deferred v2 pluggable source).
- **Every `gh` call is a subprocess.** One thin `gh` wrapper module is the only place that spawns
  processes and parses JSON; the rest of the app works on typed structs.
- **v1 board is GitHub Projects v2 only.** gegrond (markdown tickets) is explicitly out.

### Dependencies (initial `Cargo.toml`)
| crate | why |
|---|---|
| `ratatui` | TUI framework |
| `crossterm` | terminal backend + events |
| `tokio` (rt-multi-thread, macros, process, time) | async runtime, `gh` subprocess, poll timer |
| `serde` + `serde_json` | parse `gh --format json` / `gh --json` output |
| `toml` | read/write `~/.config/lazytickets/config.toml` |
| `anyhow` | error handling in app code |
| `thiserror` | typed errors in the `gh`/config layers |
| `directories` | resolve XDG config dir portably |
| `fuzzy-matcher` (nucleo or skim) | `/` live filter |

---

## 1. Module layout

```
src/
  main.rs            # arg parse, init config, launch tokio + terminal, run App
  app.rs             # App state, event loop, key dispatch, mode enum
  config/
    mod.rs           # Config struct, load/save, XDG path
    resolver.rs      # cwd git-remote → board resolution + first-run wizard
    schema.rs        # ProjectConfig, filters, presets, skill block (serde)
  gh/
    mod.rs           # the ONLY subprocess-spawning module
    project.rs       # `gh project item-list` → Vec<Item>
    issue.rs         # `gh issue view --json ...` → IssueDetail
    scope.rs         # scope check + `gh auth refresh -s project`
    write.rs         # move card status (GraphQL via gh api)
  model.rs           # domain types: Item, IssueDetail, Status, Label, Board
  ui/
    mod.rs           # top-level render(), two-pane layout
    list.rs          # left pane: filtered task list + preset tabs
    detail.rs        # right pane: desc + comments + status + mover
    filter.rs        # `/` live filter input + fuzzy match
  tmux.rs            # session detect, busy-guard, send-keys start-work flow
  poll.rs            # background 30-60s poll task + diff/reconcile
```

Build the modules roughly in the order of the milestones below, not all at once.

---

## 2. Milestones

Each milestone is independently runnable and demoable. Ship M1 fully before M2, etc.
Acceptance = the "Done when" line actually works when you run the binary.

### Progress tracker
| # | Milestone | Status | Commit |
|---|---|---|---|
| M0 | Repo + skeleton | ✅ done | `a0922bb` |
| M1 | Read-only board fetch | ✅ done | `377b2f2` |
| M2 | Detail pane (+ async loop, session cache) | ✅ done | `00dd21b`, `ee8f39d` |
| M3 | Filtering (presets + live fuzzy) | ✅ done | `7edf88e` |
| M4 | Resolver + first-run wizard | ✅ done | — |
| M5 | Start-work (`s`) — headline feature | ✅ done | — |
| M6 | Status writes | ▶ **next** | — |
| M7 | Open in browser + poll + polish | ☐ todo | — |

_First usable as the real tmux tool at **M4** (unhardcodes the board); does the headline start-work flow at **M5**._

### M0 — Repo + skeleton  *(0.5 day)* ✅
- Create **private** repo `github.com/YasarMahomedAbbas/lazytickets` (`gh repo create --private`).
- `cargo new lazytickets`, edition 2024, add `.gitignore` (`/target`, `*.log`), add deps above.
- Minimal `main.rs`: enter alternate screen, draw a "hello" ratatui frame, quit on `q`.
- Initial commit + push.
- **Done when:** `cargo run` opens a full-screen TUI that quits cleanly on `q` and restores the terminal.

### M1 — Read-only board fetch  *(1–2 days)* ✅
- `gh/project.rs`: shell `gh project item-list 6 --owner WhiteWolfStudio --format json`,
  parse into `Vec<Item>` (title, number, status, labels, assignees, url).
- `model.rs`: define `Item`, `Status`, `Label`.
- `ui/list.rs`: render items in a scrollable ratatui `List`, arrow/`j`/`k` navigation, selection highlight.
- Resolver hardcoded to travel-smart board #6 at this stage (`board {owner: WhiteWolfStudio, number: 6}`).
- Apply Nord palette (cyan focused, dim idle) from the start so later status colors slot in.
- **Done when:** launching shows the live travel-smart board as a navigable list.

### M2 — Detail pane  *(1 day)* ✅
- `gh/issue.rs`: on selection, `gh issue view <n> --json title,body,labels,comments,url,state`
  → `IssueDetail`. This is the per-issue second fetch (decided in the note).
- `ui/detail.rs`: right pane, lazygit-style split — body (wrapped), comments, status, labels, url.
- Debounce selection so fast `j`/`k` scrolling doesn't fire a fetch per row (fetch on ~150ms settle).
- Show a spinner/"loading…" while the detail fetch is in flight (async, off UI thread).
- **Done when:** moving the cursor loads and renders each ticket's full detail on the right.

### M3 — Filtering  *(1–2 days)* ✅
- `config/schema.rs`: `ProjectConfig` with `include {labels, statuses}`, `exclude_statuses`,
  `status_order`, and named `presets` (mirror `~/vault-automation/configs/*.json` format).
- Preset tabs across the top of the list pane (gh-dash style: `mine`, `refine`, `frontend`, …);
  Tab / Shift-Tab or number keys to switch.
- `ui/filter.rs`: `/` opens a transient fuzzy filter over the *current* preset's items; Esc clears.
- Sort by `status_order`, then apply include/exclude.
- **Done when:** preset tabs re-filter the list and `/` does live fuzzy narrowing on top.

### M4 — Resolver + first-run wizard  *(2 days)* ✅
> Wizard is an **in-TUI screen** (not terminal prompts); scope is **board + skill only**
> (`target_session`/`claude_subdir` deferred to M5, present as optional fields). No special-casing
> for travel-smart. Pure resolution lives in `config/resolver.rs` (unit-tested); the interactive
> wizard is split into `config/wizard.rs`. Board discovery (`list_boards`, `status_options`) is in
> `gh/project.rs`. A new project's `status_order` is seeded from the board's Status field.
- `config/resolver.rs`, try in order (stop at first hit):
  1. explicit per-folder override (path-keyed in global config)
  2. git-remote match — `git -C <cwd> rev-parse --show-toplevel` + `git remote get-url origin`,
     normalize `git@…`/https → `owner/repo`, look up the config whose `repos:` contains it → its board.
     (Worktree-proof: all travel-smart worktrees share one remote → board #6.)
  3. first-run wizard — unrecognized repo → `gh` finds candidate board(s), pick one,
     detect `.claude/skills/*`, write config + `repo → board` mapping.
- `config/mod.rs`: load/save `~/.config/lazytickets/config.toml` (via `directories`).
- Remove the M1 hardcode; board now comes from resolution.
- **Done when:** launching from any registered repo opens its board with zero prompts;
  an unknown repo runs the wizard once, then resolves instantly forever after.

### M5 — Start-work (`s`)  *(2 days)* ✅  ← headline feature
> `tmux.rs` (the only tmux-spawning module): `current_session`, `has_claude_window`, `is_busy`
> (pane-ownership check copied from `sesh-list-bells`), `start_work` (send-keys `/clear` then the
> explicit skill instruction, literal `-l` + Enter). An in-TUI `Modal` (confirm / message) captures
> input; `s` validates preconditions (real issue, linked skill, tmux session, `claude` window, not
> busy) and opens the confirm prompt. **Deferred:** label-based frontend/backend skill selection —
> M5 uses the single `skill.start` from config (the wizard links one skill); a label→skill map is a
> follow-up. Board-card auto-flip to *In progress* is M6.
- `tmux.rs`:
  - detect current session: `tmux display-message -p '#S'`; target `-t <session>:claude`.
  - **busy-guard:** read `@claude_busy` tmux session option (copy the pane-ownership check from
    `~/.local/bin/sesh-list-bells`); refuse/warn if set so we never `/clear` Claude mid-task.
  - confirm prompt → `send-keys -t <session>:claude '/clear' Enter`
    → send explicit skill instruction naming the **linked skill** from config
    (e.g. `Use the frontend-start-ticket skill for issue #365`). Deterministic, no trigger-guessing.
  - global-skill fallback (`~/.claude/skills/`) for repos without a project skill.
- Skill name comes from config (linked at wizard time; frontend/backend chosen from the issue's label).
- **Done when:** pressing `s` on a ticket clears the project's claude pane and kicks off the right skill,
  and is blocked with a warning when that session is busy.

### M6 — Status writes  *(1–2 days)*
- `gh/scope.rs`: on first write, check token scopes; if `project` missing, run
  `gh auth refresh -s project` (one-time, on the main token — no separate token).
- `gh/write.rs`: move a card's Status field via `gh api graphql` (Projects v2 is GraphQL-only).
  Need the project's Status field id (`PVTSSF_…`) + option ids — fetch/cache per board.
- Auto-flip → **In progress** on start-work (M5 hook).
- Manual column mover in the detail pane: a picker listing the board's status options.
- **Optimistic update:** mutate in-memory state + repaint immediately, fire `gh` in background, reconcile.
- **Done when:** `s` auto-moves the card to In progress, and the manual mover moves a card to any column,
  both reflected on screen instantly.

### M7 — Open in browser (`o`) + polish  *(0.5 day)*
- `o` → `gh issue view --web <n>` (or `gh project` url for project-only items).
- Background poll (`poll.rs`): `tokio` timer every 30–60s, re-fetch board off the UI thread,
  diff against current state, repaint only on a real change (no flicker). Rate limit is a non-issue.
- Help overlay (`?`), consistent keybindings, avoid tmux `C-f` and ghostty `Ctrl+Shift+*` clashes.
- **Done when:** `o` opens the ticket in the browser and external board changes appear within ~60s silently.

---

## 3. Keybindings (v1)
| key | action |
|---|---|
| `j`/`k`, `↓`/`↑` | move selection |
| `Tab`/`Shift-Tab`, `1..9` | switch preset tab |
| `/` | live fuzzy filter (Esc to clear) |
| `s` | start work (drive claude pane) |
| `m` | manual status mover |
| `o` | open in browser |
| `r` | force refresh |
| `?` | help overlay |
| `q` | quit |

Avoid: tmux prefix `C-f`, ghostty `Ctrl+Shift+*`.

## 4. Config file shape (`~/.config/lazytickets/config.toml`)
Per-project entry, modeled on `vault-automation/configs/*.json` + a `skill` block:
```toml
[[projects]]
name = "travel-smart"
repos = ["WhiteWolfStudio/travel-smart"]
board = { owner = "WhiteWolfStudio", number = 6 }
target_session = "travelsmart"      # tmux session for start-work
claude_subdir  = "Frontend"         # where claude runs
status_order = ["Refine", "Create Contract", "Ready To Implement", "In progress", "In review", "Done"]
exclude_statuses = ["Done", "Backlog"]

[projects.skill]
start = "frontend-start-ticket"     # or global ~/.claude/skills/ fallback
create = "create-ticket"

[[projects.presets]]
name = "mine"
include = { assignees = ["@me"] }
[[projects.presets]]
name = "frontend"
include = { labels = ["Frontend"], statuses = ["Refine"] }

# path-keyed overrides win over git-remote resolution
[overrides]
"/home/dracul/projects/personal/some-repo" = "travel-smart"
```

## 5. Testing approach
- **Unit:** remote-URL normalization (`git@`/https → `owner/repo`), filter/sort logic, config
  parse/round-trip, `gh` JSON → struct parsing (feed captured JSON fixtures). These are pure and cheap.
- **`gh` layer:** capture real `gh … --json` output once into `tests/fixtures/`, parse against it —
  no network in tests.
- **Manual/integration:** the tmux start-work flow and status writes are exercised by hand against a
  throwaway test card on board #6 (or a scratch project) before wiring auto-flip.

## 6. Sequencing summary
```
M0 skeleton → M1 fetch/list → M2 detail → M3 filter → M4 resolver
   → M5 start-work → M6 status writes → M7 browser + poll + polish
```
Roughly 11–14 focused days. M1–M3 are the "read" half (safe, no writes, no tmux); M5–M6 are the
"drive & write" half (needs the busy-guard and `project` scope in place first). M4 unblocks using it
across all your repos, so it's worth doing before M5 rather than after.

## 7. Deferred to v2 (do not build in v1)
Pluggable file-ticket source for gegrond · webhook push via VPS relay · AI create-issue ·
auto-authoring a skill into a skill-less repo · busy/idle status column across all projects.
