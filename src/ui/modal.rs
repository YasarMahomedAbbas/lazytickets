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

    let (title, body, border) = match modal {
        Modal::Confirm {
            issue,
            skill,
            session,
            ..
        } => (
            " Start work ",
            format!(
                "Start #{issue} with the '{skill}' skill\nin {session}:claude?\n\nThis clears the Claude pane first.\n\n[y] start    [n] cancel"
            ),
            NORD_CYAN,
        ),
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
        Modal::StatusMove { .. }
        | Modal::ProjectPick { .. }
        | Modal::Help
        | Modal::FilterBuild(_)
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
        ("Tab / S-Tab · 1-9", "switch preset tab"),
        ("/", "live fuzzy filter (Esc clears)"),
        ("f", "new saved filter (preset)"),
        ("e", "edit the active filter"),
        ("d", "delete the active filter"),
        ("s", "start work (drive claude pane)"),
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
