//! First-run wizard: an in-TUI screen shown when the current repo isn't in the
//! config. Discovers the owner's boards via `gh`, lets the user pick one, detects
//! a start-work skill, writes a new project entry, and returns it. Every launch
//! after resolves instantly via `resolver`.

use super::Config;
use super::schema::{Board, Filter, Preset, ProjectConfig, Skill};
use crate::gh;
use crate::ui::{NORD_CYAN, NORD_DIM};
use anyhow::{Context, Result, bail};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use std::path::Path;
use tokio::sync::mpsc;

/// Where the wizard reads keystrokes from.
///
/// At first run (`main`) the wizard owns the terminal, so it reads events
/// directly. When launched from inside the running app ("Add a board…") the main
/// loop already has a background thread draining `event::read()` into a channel —
/// reading the terminal here too would make the two compete and drop every other
/// keypress. In that case the wizard pulls from the same channel instead.
pub enum Input<'a> {
    Terminal,
    Channel(&'a mut mpsc::UnboundedReceiver<Event>),
}

impl Input<'_> {
    /// Block until the next key *press* (release/repeat events are skipped).
    async fn next_key(&mut self) -> Result<KeyCode> {
        loop {
            let ev = match self {
                Input::Terminal => tokio::task::spawn_blocking(event::read).await??,
                Input::Channel(rx) => rx
                    .recv()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("input channel closed"))?,
            };
            if let Event::Key(k) = ev
                && k.kind == KeyEventKind::Press
            {
                return Ok(k.code);
            }
        }
    }
}

/// Run the wizard for the (unrecognised) repo at `cwd`. On success writes the new
/// project to `config` on disk and returns it. Cancelling or a missing remote is
/// an error the caller surfaces after restoring the terminal.
/// `bind_repo` controls whether the new project claims the current repo in its
/// `repos:` (so cwd resolves to it on future launches). True for the first-run
/// wizard; false for the in-TUI "Add a board…" flow, which registers a
/// switch-only board that must not hijack another project's cwd resolution.
pub async fn run(
    terminal: &mut DefaultTerminal,
    repo: Option<String>,
    cwd: &Path,
    mut config: Config,
    bind_repo: bool,
    mut input: Input<'_>,
) -> Result<ProjectConfig> {
    let repo = match repo {
        Some(r) => r,
        None => {
            show_message(
                terminal,
                &mut input,
                "No GitHub remote found here.\n\nlazytickets resolves a board from the repo's `origin` remote, so run it from \
                 inside a GitHub repository.\n\nPress any key to exit.",
            )
            .await?;
            bail!("no GitHub remote in the current directory");
        }
    };
    let owner = repo.split('/').next().unwrap_or(&repo).to_string();

    draw_loading(terminal, &format!("Loading boards for {owner}…"))?;
    let boards = gh::project::list_boards(&owner)
        .await
        .with_context(|| format!("listing project boards for {owner}"))?;
    if boards.is_empty() {
        show_message(
            terminal,
            &mut input,
            &format!(
                "No open project boards found for {owner}.\n\nCreate one on GitHub, then relaunch.\n\nPress any key to exit."
            ),
        )
        .await?;
        bail!("no project boards found for {owner}");
    }

    // Screen 1: pick a board.
    let labels: Vec<String> = boards
        .iter()
        .map(|b| format!("#{}  {}", b.number, b.title))
        .collect();
    let board = match pick(
        terminal,
        &mut input,
        &format!("Setup: {repo}"),
        "Pick a board:",
        &labels,
        false,
    )
    .await?
    {
        Some(i) => &boards[i],
        None => bail!("wizard cancelled"),
    };

    // Seed status_order from the board's Status column; default excludes to any
    // Done/Backlog columns that exist.
    draw_loading(terminal, "Reading board columns…")?;
    let status_order = gh::project::status_options(&owner, board.number)
        .await
        .unwrap_or_default();
    let exclude_statuses = ["Done", "Backlog"]
        .into_iter()
        .filter(|d| status_order.iter().any(|s| s.eq_ignore_ascii_case(d)))
        .map(str::to_string)
        .collect();

    // Screen 2 (only if candidates): pick a start-work skill.
    let skills = detect_skills(cwd);
    let start = match skills.len() {
        0 => None,
        1 => Some(skills[0].clone()),
        _ => pick(
            terminal,
            &mut input,
            &format!("Setup: {repo}"),
            "Pick a start-work skill (s to skip):",
            &skills,
            true,
        )
        .await?
        .map(|i| skills[i].clone()),
    };

    // Presets: `all`, plus `mine` bound to the resolved viewer login so it filters
    // without special `@me` handling.
    let mut presets = vec![Preset {
        name: "all".into(),
        include: Filter::default(),
    }];
    if let Ok(login) = gh::viewer_login().await
        && !login.is_empty()
    {
        presets.push(Preset {
            name: "mine".into(),
            include: Filter {
                assignees: vec![login],
                ..Default::default()
            },
        });
    }

    // Name the project after the *board* (what the user picked), not the repo —
    // otherwise multiple boards added from one repo all collide on the repo name.
    // Fall back to the repo name for an untitled board, and disambiguate any
    // collision with an existing entry by appending the board number.
    let base = match board.title.trim() {
        "" => repo.split('/').nth(1).unwrap_or(&repo),
        title => title,
    };
    let mut name = base.to_string();
    if config.projects.iter().any(|p| p.name == name) {
        name = format!("{name} #{}", board.number);
    }

    let project = ProjectConfig {
        name,
        repos: if bind_repo {
            vec![repo.clone()]
        } else {
            vec![]
        },
        board: Board {
            owner,
            number: board.number,
        },
        target_session: None,
        claude_subdir: None,
        skill: Skill {
            start,
            create: None,
        },
        status_order,
        exclude_statuses,
        presets,
    };

    config.projects.push(project.clone());
    config.save().context("saving the new project to config")?;
    Ok(project)
}

/// Collect skill names from the nearest `.claude/skills` walking up from `cwd`,
/// then any global `~/.claude/skills` not already found. Local wins on ties.
fn detect_skills(cwd: &Path) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();

    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let skills = d.join(".claude").join("skills");
        if skills.is_dir() {
            names.extend(skill_dir_names(&skills));
            break;
        }
        dir = d.parent();
    }

    if let Some(base) = directories::BaseDirs::new() {
        let global = base.home_dir().join(".claude").join("skills");
        for n in skill_dir_names(&global) {
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }

    names
}

/// Immediate subdirectory names of `dir` (each is a skill), sorted.
fn skill_dir_names(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.path().is_dir()
                && let Some(name) = e.file_name().to_str()
            {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names
}

// --- ratatui plumbing ---------------------------------------------------------

/// A single-select list screen. Returns the chosen index, or `None` on
/// cancel/skip (`q`/`Esc`, or `s` when `skippable`).
async fn pick(
    terminal: &mut DefaultTerminal,
    input: &mut Input<'_>,
    title: &str,
    prompt: &str,
    items: &[String],
    skippable: bool,
) -> Result<Option<usize>> {
    let mut state = ListState::default();
    state.select(Some(0));
    loop {
        terminal.draw(|f| draw_pick(f, title, prompt, items, skippable, &mut state))?;
        match input.next_key().await? {
            KeyCode::Char('j') | KeyCode::Down => step(&mut state, items.len(), 1),
            KeyCode::Char('k') | KeyCode::Up => step(&mut state, items.len(), -1),
            KeyCode::Enter => return Ok(state.selected()),
            KeyCode::Char('s') if skippable => return Ok(None),
            KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
            _ => {}
        }
    }
}

/// Move the selection by `delta`, clamped to `[0, len)`.
fn step(state: &mut ListState, len: usize, delta: isize) {
    if len == 0 {
        return;
    }
    let cur = state.selected().unwrap_or(0) as isize;
    let next = (cur + delta).clamp(0, len as isize - 1) as usize;
    state.select(Some(next));
}

fn draw_pick(
    f: &mut Frame,
    title: &str,
    prompt: &str,
    items: &[String],
    skippable: bool,
    state: &mut ListState,
) {
    let area = f.area();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NORD_CYAN))
        .title(format!(" {title} "));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [prompt_area, list_area, hint_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    f.render_widget(
        Paragraph::new(prompt).style(Style::default().fg(NORD_DIM)),
        prompt_area,
    );

    let rows: Vec<ListItem> = items
        .iter()
        .map(|s| ListItem::new(Line::from(s.as_str())))
        .collect();
    let list = List::new(rows)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, list_area, state);

    let hint = if skippable {
        "j/k move · Enter select · s skip · q cancel"
    } else {
        "j/k move · Enter select · q cancel"
    };
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(NORD_DIM)),
        hint_area,
    );
}

/// A one-shot full-screen status frame (no input).
fn draw_loading(terminal: &mut DefaultTerminal, msg: &str) -> Result<()> {
    terminal.draw(|f| {
        let p = Paragraph::new(msg)
            .style(Style::default().fg(NORD_DIM))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(NORD_CYAN))
                    .title(" lazytickets "),
            );
        f.render_widget(p, f.area());
    })?;
    Ok(())
}

/// Show a message and wait for any keypress.
async fn show_message(
    terminal: &mut DefaultTerminal,
    input: &mut Input<'_>,
    msg: &str,
) -> Result<()> {
    terminal.draw(|f| {
        let p = Paragraph::new(msg)
            .style(Style::default().fg(NORD_DIM))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(NORD_CYAN))
                    .title(" lazytickets "),
            );
        f.render_widget(p, f.area());
    })?;
    let _ = input.next_key().await?;
    Ok(())
}
