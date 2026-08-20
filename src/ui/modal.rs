//! Centered overlay for the start-work flow (confirm prompt + messages).

use crate::app::{FilterDraft, Modal};
use crate::ui::{NORD_AMBER, NORD_CYAN, NORD_DIM, NORD_GREEN};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub fn render(frame: &mut Frame, modal: &Modal) {
    // The pickers render their own selectable lists; the rest are text popups.
    if let Modal::StatusMove {
        options, selected, ..
    } = modal
    {
        render_mover(frame, options, *selected);
        return;
    }
    if let Modal::ProjectPick { names, selected } = modal {
        render_project_pick(frame, names, *selected);
        return;
    }
    if matches!(modal, Modal::Help) {
        render_help(frame);
        return;
    }
    if let Modal::FilterBuild(draft) = modal {
        render_builder(frame, draft);
        return;
    }
    if let Modal::Create(draft) = modal {
        render_create(frame, draft);
        return;
    }

    if let Modal::Confirm {
        issue,
        skill,
        session,
        prompt,
        editing,
        ..
    } = modal
    {
        render_start(
            frame,
            " Start work ",
            NORD_CYAN,
            vec![
                format!("Start #{issue} with the '{skill}' skill in {session}:claude."),
                "This clears the Claude pane first.".into(),
            ],
            prompt,
            *editing,
        );
        return;
    }
    if let Modal::WorktreeConfirm {
        issue,
        skill,
        session,
        path,
        base,
        subdir,
        bootstrap,
        prompt,
        editing,
        ..
    } = modal
    {
        let mut info = vec![format!("Fork from:  {base}")];
        // Only mention the subdir when one is configured; the common single-root
        // repo shouldn't carry an empty "start in" line.
        if let Some(sd) = subdir {
            info.push(format!("Start in:   {sd}/"));
        }
        if let Some(b) = bootstrap {
            info.push(format!("Bootstrap:  {b}"));
        }
        info.push(String::new());
        info.push(format!("Create worktree {path}"));
        info.push(format!(
            "and start #{issue} with '{skill}' in a new session '{session}'."
        ));
        render_start(frame, " Start in worktree ", NORD_GREEN, info, prompt, *editing);
        return;
    }

    let (title, body, border) = match modal {
        Modal::ConfirmDelete { name, .. } => (
            " Delete filter ",
            format!(
                "Delete the '{name}' filter?\n\nThis removes it from your config.\n\n[y] delete    [n] cancel"
            ),
            NORD_AMBER,
        ),
        Modal::Message(msg) => (
            " lazytickets ",
            format!("{msg}\n\n[any key] dismiss"),
            NORD_AMBER,
        ),
        Modal::Confirm { .. }
        | Modal::WorktreeConfirm { .. }
        | Modal::StatusMove { .. }
        | Modal::ProjectPick { .. }
        | Modal::Help
        | Modal::FilterBuild(_)
        | Modal::Create(_)
        | Modal::None => return,
    };

    let area = centered(60, 11, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: true }).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border)),
        ),
        area,
    );
}

/// The start-work confirms: a few lines of context, then the prompt that will be
/// sent to Claude. Plain y/n by default; `e` turns the prompt into a text field
/// (prefilled, so you add to it rather than retype) — Enter starts, Esc goes back.
fn render_start(
    frame: &mut Frame,
    title: &str,
    border: ratatui::style::Color,
    info: Vec<String>,
    prompt: &str,
    editing: bool,
) {
    use ratatui::text::Span;

    let dim = Style::default().fg(NORD_DIM);
    let amber = Style::default().fg(NORD_AMBER);
    let cyan_bold = Style::default().fg(NORD_CYAN).add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line> = info.into_iter().map(Line::raw).collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled("Prompt sent to Claude:", amber));
    let shown = if editing {
        Span::styled(format!("{prompt}▏"), cyan_bold)
    } else if prompt.is_empty() {
        Span::styled("(empty)", dim)
    } else {
        Span::raw(prompt.to_string())
    };
    lines.push(Line::from(vec![Span::raw("  "), shown]));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        if editing {
            "[Enter] start    [Esc] done editing    [Ctrl+U] clear"
        } else {
            "[y] start    [e] edit prompt    [n] cancel"
        },
        dim,
    ));

    // Long paths and prompts wrap inside the box; size it from the wrapped row
    // count at the real width so nothing clips.
    let width = centered(70, 1, frame.area()).width;
    let inner = width.saturating_sub(2).max(1) as usize;
    let rows: usize = lines
        .iter()
        .map(|l| (l.width().max(1)).div_ceil(inner))
        .sum();
    let height = (rows as u16 + 2).min(frame.area().height);

    let area = centered(70, height, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border)),
        ),
        area,
    );
}

fn render_mover(frame: &mut Frame, options: &[String], selected: usize) {
    let mut lines: Vec<Line> = options
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if i == selected {
                Line::styled(
                    format!("▶ {name}"),
                    Style::default().add_modifier(Modifier::REVERSED),
                )
            } else {
                Line::raw(format!("  {name}"))
            }
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "j/k move · Enter set · Esc cancel",
        Style::default().fg(NORD_AMBER),
    ));

    let height = (lines.len() as u16 + 2).min(frame.area().height);
    let area = centered(50, height, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" Move to status ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(NORD_CYAN)),
        ),
        area,
    );
}

/// The project switcher: configured projects plus a trailing "Add a board…" row
/// (highlighted when `selected == names.len()`).
fn render_project_pick(frame: &mut Frame, names: &[String], selected: usize) {
    let mut rows: Vec<String> = names.to_vec();
    rows.push("＋ Add a board…".to_string());

    let mut lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if i == selected {
                Line::styled(
                    format!("▶ {name}"),
                    Style::default().add_modifier(Modifier::REVERSED),
                )
            } else {
                Line::raw(format!("  {name}"))
            }
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "j/k move · Enter open · Esc cancel",
        Style::default().fg(NORD_AMBER),
    ));

    let height = (lines.len() as u16 + 2).min(frame.area().height);
    let area = centered(50, height, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" Switch project ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(NORD_CYAN)),
        ),
        area,
    );
}

// Nerd Font glyphs (Font Awesome range) used by the filter builder.
const G_FILTER: &str = "\u{f0b0}"; // funnel — modal title
const G_PENCIL: &str = "\u{f040}"; // editable name field
const G_CHECK_ON: &str = "\u{f14a}"; // checked box
const G_CHECK_OFF: &str = "\u{f096}"; // empty box
const G_STATUS: &str = "\u{f0db}"; // columns — Status group
const G_TAG: &str = "\u{f02c}"; // tags — Labels group
const G_USER: &str = "\u{f0c0}"; // users — Assignees group
const G_CARET: &str = "\u{f0da}"; // focus pointer
const G_REPO: &str = "\u{f1c0}"; // database/repo — create form target
const G_DOC: &str = "\u{f036}"; // align-left — description field
const G_PLUS: &str = "\u{f067}"; // plus — create button / modal title

/// The filter builder: an editable name field over three glyph-labelled groups
/// of checkboxes, with a live summary of the view it will produce. `focus == 0`
/// is the name field; `1..` walk the option rows in group order, marked by a
/// caret. A green check means the value is included (shown).
fn render_builder(frame: &mut Frame, draft: &FilterDraft) {
    use ratatui::text::Span;

    let dim = Style::default().fg(NORD_DIM);
    let amber = Style::default().fg(NORD_AMBER);
    let green = Style::default().fg(NORD_GREEN);
    let cyan = Style::default().fg(NORD_CYAN);
    let cyan_bold = cyan.add_modifier(Modifier::BOLD);

    // A caret before the focused row, or blank padding to keep columns aligned.
    let ptr = |focused: bool| {
        if focused {
            Span::styled(format!("{G_CARET} "), cyan)
        } else {
            Span::raw("  ")
        }
    };

    let mut lines: Vec<Line> = vec![Line::raw("")];

    // --- name field (focus 0) ---
    let name_focused = draft.focus == 0;
    let name_span = if draft.name.is_empty() && !name_focused {
        Span::styled("unnamed", dim)
    } else if name_focused {
        Span::styled(format!("{}▏", draft.name), cyan_bold)
    } else {
        Span::styled(draft.name.clone(), cyan)
    };
    lines.push(Line::from(vec![
        ptr(name_focused),
        Span::styled(
            format!("{G_PENCIL}  name  "),
            if name_focused { amber } else { dim },
        ),
        name_span,
    ]));
    lines.push(Line::raw(""));

    // --- option groups ---
    let groups = [
        (G_STATUS, "Status", &draft.statuses),
        (G_TAG, "Labels", &draft.labels),
        (G_USER, "Assignees", &draft.assignees),
    ];
    let mut row = 1usize;
    for (glyph, title, items) in groups {
        if items.is_empty() {
            continue;
        }
        let checked = items.iter().filter(|(_, on)| *on).count();
        let mut header = vec![
            Span::raw("  "),
            Span::styled(
                format!("{glyph}  {title}"),
                amber.add_modifier(Modifier::BOLD),
            ),
        ];
        if checked > 0 {
            header.push(Span::styled(format!("   {G_CHECK_ON} {checked}"), green));
        }
        lines.push(Line::from(header));

        for (value, on) in items {
            let focused = draft.focus == row;
            let (box_glyph, box_style) = if *on {
                (G_CHECK_ON, green)
            } else {
                (G_CHECK_OFF, dim)
            };
            let value_style = if focused {
                cyan_bold
            } else if *on {
                Style::default()
            } else {
                dim
            };
            lines.push(Line::from(vec![
                Span::raw("   "),
                ptr(focused),
                Span::styled(format!("{box_glyph}  "), box_style),
                Span::styled(value.clone(), value_style),
            ]));
            row += 1;
        }
        lines.push(Line::raw(""));
    }

    if draft.option_count() == 0 {
        lines.push(Line::styled(
            "   nothing on this board to filter on yet",
            dim,
        ));
        lines.push(Line::raw(""));
    }

    // --- live summary of the view this filter produces ---
    let seg = |glyph: &str, items: &[(String, bool)]| -> Option<String> {
        let picked: Vec<&str> = items
            .iter()
            .filter(|(_, on)| *on)
            .map(|(v, _)| v.as_str())
            .collect();
        (!picked.is_empty()).then(|| format!("{glyph} {}", picked.join(", ")))
    };
    let parts: Vec<String> = [
        seg(G_STATUS, &draft.statuses),
        seg(G_TAG, &draft.labels),
        seg(G_USER, &draft.assignees),
    ]
    .into_iter()
    .flatten()
    .collect();
    let summary = if parts.is_empty() {
        Span::styled("everything on the board", amber)
    } else {
        Span::styled(parts.join("    "), green)
    };
    lines.push(Line::from(vec![Span::styled("  showing  ", dim), summary]));
    lines.push(Line::raw(""));

    // --- key hints ---
    lines.push(Line::styled(
        "  space toggle · j/k move · h/l section · ↵ save · esc cancel",
        amber,
    ));

    let height = (lines.len() as u16 + 2).min(frame.area().height);
    let area = centered(66, height, frame.area());
    frame.render_widget(Clear, area);
    // No soft-wrap: each Line is its own row and clips at the border, so a long
    // summary or name never spills to column 0.
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(Line::from(format!(" {G_FILTER}  New filter ")))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(NORD_CYAN)),
        ),
        area,
    );
}

/// The create-ticket form: an editable title + multi-line description over two
/// cyclable optional fields (label, status), ending in a Create button. `focus`
/// walks `[title, body, label, status, button]`; the focused field is marked by a
/// caret and cyan text.
fn render_create(frame: &mut Frame, draft: &crate::app::CreateDraft) {
    use ratatui::text::Span;

    let dim = Style::default().fg(NORD_DIM);
    let amber = Style::default().fg(NORD_AMBER);
    let green = Style::default().fg(NORD_GREEN);
    let cyan = Style::default().fg(NORD_CYAN);
    let cyan_bold = cyan.add_modifier(Modifier::BOLD);

    let ptr = |focused: bool| {
        if focused {
            Span::styled(format!("{G_CARET} "), cyan)
        } else {
            Span::raw("  ")
        }
    };
    // A labelled single-line text field with a block cursor when focused.
    let text_row = |glyph: &str, label: &str, value: &str, focused: bool, empty_hint: &str| {
        let value_span = if value.is_empty() && !focused {
            Span::styled(empty_hint.to_string(), dim)
        } else if focused {
            Span::styled(format!("{value}▏"), cyan_bold)
        } else {
            Span::styled(value.to_string(), Style::default())
        };
        Line::from(vec![
            ptr(focused),
            Span::styled(
                format!("{glyph}  {label:<12}"),
                if focused { amber } else { dim },
            ),
            value_span,
        ])
    };

    let mut lines: Vec<Line> = vec![Line::raw("")];

    // Target repo (not editable) — makes the create destination unambiguous.
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{G_REPO}  {}", draft.repo), dim),
    ]));
    lines.push(Line::raw(""));

    // Title (focus 0).
    lines.push(text_row(
        G_PENCIL,
        "title",
        &draft.title,
        draft.focus == 0,
        "required",
    ));
    lines.push(Line::raw(""));

    // Description (focus 1) — multi-line, rendered under its label.
    let body_focused = draft.focus == 1;
    lines.push(Line::from(vec![
        ptr(body_focused),
        Span::styled(
            format!("{G_DOC}  description"),
            if body_focused { amber } else { dim },
        ),
    ]));
    let body_lines: Vec<&str> = if draft.body.is_empty() {
        vec![""]
    } else {
        draft.body.split('\n').collect()
    };
    let last = body_lines.len() - 1;
    for (i, seg) in body_lines.iter().enumerate() {
        let is_last = i == last;
        let span = if draft.body.is_empty() && !body_focused {
            Span::styled("optional", dim)
        } else if body_focused && is_last {
            Span::styled(format!("{seg}▏"), cyan_bold)
        } else {
            Span::styled((*seg).to_string(), Style::default())
        };
        lines.push(Line::from(vec![Span::raw("      "), span]));
    }
    lines.push(Line::raw(""));

    // Label + status (focus 2, 3) — cyclable optional pickers.
    let picker_row = |glyph: &str, label: &str, value: &str, focused: bool| {
        let value_style = if focused {
            cyan_bold
        } else if value == "(none)" {
            dim
        } else {
            green
        };
        Line::from(vec![
            ptr(focused),
            Span::styled(
                format!("{glyph}  {label:<12}"),
                if focused { amber } else { dim },
            ),
            Span::styled(format!("‹ {value} ›"), value_style),
        ])
    };
    lines.push(picker_row(
        G_TAG,
        "label",
        draft.label_display(),
        draft.focus == 2,
    ));
    lines.push(picker_row(
        G_STATUS,
        "status",
        draft.status_display(),
        draft.focus == 3,
    ));
    lines.push(Line::raw(""));

    // Create button (focus 4).
    let btn_focused = draft.focus == 4;
    let btn = if btn_focused {
        Span::styled(
            format!("  {G_PLUS}  Create ticket  "),
            Style::default()
                .fg(NORD_GREEN)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
        )
    } else {
        Span::styled(format!("  {G_PLUS}  Create ticket  "), green)
    };
    lines.push(Line::from(vec![ptr(btn_focused), btn]));
    lines.push(Line::raw(""));

    lines.push(Line::styled(
        "  tab/↑↓ move · ‹h/l›/space cycle · ↵ newline·create · esc cancel",
        amber,
    ));

    let height = (lines.len() as u16 + 2).min(frame.area().height);
    let area = centered(66, height, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(Line::from(format!(" {G_PLUS}  New ticket ")))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(NORD_CYAN)),
        ),
        area,
    );
}

/// A transient centered notice (e.g. "Loading…") drawn over the current frame
/// while a blocking board fetch is in flight. No dismiss hint — it's not modal.
pub fn render_notice(frame: &mut Frame, msg: &str) {
    let area = centered(50, 3, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(msg).wrap(Wrap { trim: true }).block(
            Block::default()
                .title(" lazytickets ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(NORD_CYAN)),
        ),
        area,
    );
}

fn render_help(frame: &mut Frame) {
    let rows = [
        ("j / k · ↓ / ↑", "move selection"),
        ("J / K · C-d / C-u", "scroll detail (half-page)"),
        ("h / l · Tab · 1-9", "switch preset tab"),
        ("/", "live fuzzy filter (Esc clears)"),
        ("c", "create a ticket"),
        ("L", "toggle inline labels"),
        ("f", "new saved filter (preset)"),
        ("e", "edit the active filter"),
        ("d", "delete the active filter"),
        ("s", "start work (drive claude pane)"),
        ("t", "start work in a new worktree + session"),
        ("m", "move status column"),
        ("p", "switch project"),
        ("o", "open in browser"),
        ("r", "force refresh"),
        ("?", "help"),
        ("q", "quit"),
    ];
    let mut lines: Vec<Line> = rows
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                ratatui::text::Span::styled(format!("{k:<20}"), Style::default().fg(NORD_CYAN)),
                ratatui::text::Span::raw(*d),
            ])
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "[any key] dismiss",
        Style::default().fg(NORD_AMBER),
    ));

    let height = (lines.len() as u16 + 2).min(frame.area().height);
    let area = centered(60, height, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" Keybindings ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(NORD_CYAN)),
        ),
        area,
    );
}

/// A box `percent_x` wide and `height` tall, centered in `area`.
fn centered(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = area.width * percent_x / 100;
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height: height.min(area.height),
    }
}
