//! Left pane: preset tabs, the filtered task list, and the live-filter / hint row.

use crate::app::{App, InputMode};
use crate::ui::{
    NORD_AMBER, NORD_BLUE, NORD_CYAN, NORD_GREEN, NORD_MUTED, NORD_PURPLE, NORD_RED, NORD_SEL,
    NORD_TEXT,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, HighlightSpacing, List, ListItem, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// Nerd Font glyphs (Font Awesome range), matching the filter builder's vocabulary.
const G_BOARD: &str = "\u{f0db}"; // columns — board title
const G_FILTER: &str = "\u{f0b0}"; // funnel — preset tabs
const G_TAG: &str = "\u{f02c}"; // tags — inline labels
const G_SEARCH: &str = "\u{f002}"; // magnifier — live filter
const G_ON: &str = "\u{f14a}"; // checked box — labels toggle on
const G_OFF: &str = "\u{f096}"; // empty box — labels toggle off
// Status markers: shape encodes progress, colour encodes which column.
const M_TODO: &str = "\u{f10c}"; // hollow circle — not started
const M_ACTIVE: &str = "\u{f111}"; // filled circle — in flight
const M_DONE: &str = "\u{f058}"; // check-circle — done
const M_BLOCKED: &str = "\u{f057}"; // times-circle — blocked

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    // tabs (1) · list (rest) · footer (1): filter input while filtering, else hints.
    let chunks =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
            .split(area);

    render_tabs(frame, chunks[0], app);
    render_list(frame, chunks[1], app);
    render_footer(frame, chunks[2], app);
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![Span::styled(
        format!(" {G_FILTER}  "),
        Style::default().fg(NORD_MUTED),
    )];
    for i in 0..app.preset_count() {
        let name = app.preset_name(i);
        if i == app.active_preset {
            // Active preset as a filled pill so the current view is unmistakable.
            spans.push(Span::styled(
                format!(" {name} "),
                Style::default()
                    .bg(NORD_CYAN)
                    .fg(Color::Rgb(0x2e, 0x34, 0x40))
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {name} "),
                Style::default().fg(NORD_MUTED),
            ));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_list(frame: &mut Frame, area: Rect, app: &mut App) {
    // Text width per row = inside the border, minus the 2-col highlight gutter and
    // a 1-col right margin so the status word doesn't hug the border.
    let content_w = (area.width as usize).saturating_sub(2 + 2 + 1);

    let rows: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&i| row(&app.items[i], content_w, app.show_labels))
        .collect();

    let title = Line::from(vec![
        Span::styled(format!(" {G_BOARD}  "), Style::default().fg(NORD_CYAN)),
        Span::styled(
            app.config.name.clone(),
            Style::default().fg(NORD_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  #{}  ", app.config.board.number),
            Style::default().fg(NORD_MUTED),
        ),
        Span::styled(
            app.visible.len().to_string(),
            Style::default().fg(NORD_AMBER).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" tickets ", Style::default().fg(NORD_MUTED)),
    ]);

    let list = List::new(rows)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(NORD_CYAN)),
        )
        .highlight_style(Style::default().bg(NORD_SEL).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ")
        .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

/// One list row: `● #365  Title ……  tag labels   Status`, with the status text
/// (and labels, when shown) flushed to the right edge and the title truncated to
/// fill the gap.
fn row(it: &crate::model::Item, content_w: usize, show_labels: bool) -> ListItem<'static> {
    let status = it.status.as_deref().unwrap_or("");
    let (scolor, marker) = status_marker(status);

    const MARKER_W: usize = 2; // glyph + space
    const NUM_W: usize = 5; // "#365 " (4-wide field + space)

    // Right cluster: optional labels, then the status word.
    let mut right: Vec<Span<'static>> = Vec::new();
    let mut right_w = 0usize;
    if show_labels && !it.labels.is_empty() {
        let (txt, w) = truncate(&it.labels.join(", "), (content_w / 3).max(8));
        right.push(Span::styled(
            format!("{G_TAG} "),
            Style::default().fg(NORD_PURPLE),
        ));
        right.push(Span::styled(
            format!("{txt}  "),
            Style::default().fg(NORD_PURPLE),
        ));
        right_w += 2 + w + 2;
    }
    if !status.is_empty() {
        let (txt, w) = truncate(status, (content_w / 3).max(6));
        right.push(Span::styled(
            txt,
            Style::default().fg(scolor).add_modifier(Modifier::BOLD),
        ));
        right_w += w;
    }

    // Title takes whatever the left/right clusters leave, keeping ≥1 col of gap.
    let title_avail = content_w.saturating_sub(MARKER_W + NUM_W + right_w + 1);
    let (title_txt, title_w) = truncate(&it.title, title_avail);

    let gap = content_w
        .saturating_sub(MARKER_W + NUM_W + title_w + right_w)
        .max(1);

    let num_color = if it.number.is_some() {
        NORD_BLUE
    } else {
        NORD_MUTED
    };

    let mut spans = vec![
        Span::styled(format!("{marker} "), Style::default().fg(scolor)),
        Span::styled(
            format!("{:>4} ", it.number_label()),
            Style::default().fg(num_color),
        ),
        Span::styled(title_txt, Style::default().fg(NORD_TEXT)),
        Span::raw(" ".repeat(gap)),
    ];
    spans.extend(right);
    ListItem::new(Line::from(spans))
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let filtering = app.input_mode == InputMode::Filter || !app.filter_query.is_empty();
    let line = if filtering {
        filter_line(app)
    } else {
        hint_line(app)
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn filter_line(app: &App) -> Line<'static> {
    let active = app.input_mode == InputMode::Filter;
    let mut spans = vec![
        Span::styled(format!(" {G_SEARCH} "), Style::default().fg(NORD_CYAN)),
        Span::styled(app.filter_query.clone(), Style::default().fg(NORD_TEXT)),
    ];
    if active {
        spans.push(Span::styled("▏", Style::default().fg(NORD_CYAN)));
    } else {
        spans.push(Span::styled(
            "  esc clears",
            Style::default().fg(NORD_MUTED),
        ));
    }
    Line::from(spans)
}

/// The resting footer: a compact key legend that also reflects the labels toggle.
fn hint_line(app: &App) -> Line<'static> {
    let key = |k: &str, d: &str| {
        vec![
            Span::styled(k.to_string(), Style::default().fg(NORD_CYAN)),
            Span::styled(format!(" {d}"), Style::default().fg(NORD_MUTED)),
        ]
    };
    let sep = || Span::styled("   ", Style::default().fg(NORD_MUTED));

    let (glyph, style) = if app.show_labels {
        (G_ON, Style::default().fg(NORD_GREEN))
    } else {
        (G_OFF, Style::default().fg(NORD_MUTED))
    };

    let mut spans = vec![Span::raw(" ")];
    spans.extend(key("/", "filter"));
    spans.push(sep());
    // Labels toggle carries its own on/off glyph so the state is readable at rest.
    spans.push(Span::styled("l ", Style::default().fg(NORD_CYAN)));
    spans.push(Span::styled(format!("{glyph} labels"), style));
    spans.push(sep());
    spans.extend(key("s", "start"));
    spans.push(sep());
    spans.extend(key("m", "move"));
    spans.push(sep());
    spans.extend(key("?", "help"));
    Line::from(spans)
}

/// Marker glyph + colour for a status column. Shape encodes progress (hollow →
/// filled → check), colour encodes the kind, so a board's columns read at a glance
/// regardless of the exact status names a project uses.
fn status_marker(status: &str) -> (Color, &'static str) {
    let s = status.to_ascii_lowercase();
    if s.contains("progress") || s.contains("doing") || s.contains("wip") {
        (NORD_AMBER, M_ACTIVE)
    } else if s.contains("done")
        || s.contains("closed")
        || s.contains("complete")
        || s.contains("ship")
        || s.contains("merged")
    {
        (NORD_GREEN, M_DONE)
    } else if s.contains("review")
        || s.contains("feedback")
        || s.contains("test")
        || s.contains("qa")
        || s.contains("approv")
    {
        (NORD_PURPLE, M_ACTIVE)
    } else if s.contains("block") || s.contains("hold") || s.contains("stuck") {
        (NORD_RED, M_BLOCKED)
    } else if s.is_empty() {
        (NORD_MUTED, M_ACTIVE)
    } else {
        // todo / backlog / refine / ready / triage / new / future / create …
        (NORD_BLUE, M_TODO)
    }
}

/// Truncate `s` to at most `max` display columns, appending `…` when clipped.
/// Returns the (possibly shortened) string and its actual display width.
fn truncate(s: &str, max: usize) -> (String, usize) {
    let w = UnicodeWidthStr::width(s);
    if w <= max {
        return (s.to_string(), w);
    }
    if max == 0 {
        return (String::new(), 0);
    }
    let budget = max - 1; // reserve a column for the ellipsis
    let mut out = String::new();
    let mut acc = 0;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if acc + cw > budget {
            break;
        }
        out.push(ch);
        acc += cw;
    }
    out.push('…');
    (out, acc + 1)
}
