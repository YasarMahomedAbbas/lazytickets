//! Left pane: preset tabs, the status-grouped task list, and the live-filter /
//! hint row.

use crate::app::{App, Group, InputMode, Row};
use crate::model::Item;
use crate::ui::{
    NORD_AMBER, NORD_BLUE, NORD_CYAN, NORD_GREEN, NORD_MUTED, NORD_PURPLE, NORD_SEL, NORD_TEXT,
    status_marker,
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
/// Sitemap — a parent card that has sub-issues on the board.
pub const G_PARENT: &str = "\u{f0e8}";
// Fold state of a status group header.
const G_OPEN: &str = "\u{25be}"; // ▾
const G_FOLDED: &str = "\u{25b8}"; // ▸
// Sub-issue rolled up under the parent card above it: mid / last child.
const NEST_MID: &str = "\u{251c} "; // ├
const NEST_END: &str = "\u{2514} "; // └

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    // tabs (1) · list (rest) · footer (1): filter input while filtering, else hints.
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
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
    spans.push(Span::styled("  H/L", Style::default().fg(NORD_MUTED)));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_list(frame: &mut Frame, area: Rect, app: &mut App) {
    // Text width per row = inside the border, minus the 2-col highlight gutter and
    // a 1-col right margin so the status word doesn't hug the border.
    let content_w = (area.width as usize).saturating_sub(2 + 2 + 1);

    let rows: Vec<ListItem> = app
        .rows
        .iter()
        .enumerate()
        .map(|(r, row)| match *row {
            Row::Header(g) => header(&app.groups[g], content_w),
            Row::Item(i) => {
                // A nested card is the last of its siblings when the next row
                // isn't another nested card.
                let nest = if app.nested.contains(&i) {
                    let more =
                        matches!(app.rows.get(r + 1), Some(Row::Item(n)) if app.nested.contains(n));
                    Some(more)
                } else {
                    None
                };
                let kids = app.children_of(i);
                let done = kids
                    .iter()
                    .filter(|&&k| is_done(app.items[k].status.as_deref()))
                    .count();
                let family = (!kids.is_empty()).then_some((done, kids.len()));
                card(&app.items[i], content_w, app.show_labels, nest, family)
            }
        })
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

/// Whether a status reads as finished, for the parent's `done/total` badge.
fn is_done(status: Option<&str>) -> bool {
    status.is_some_and(|s| status_marker(s).0 == NORD_GREEN)
}

/// A status group header: `▾ ● In progress ────────── 4`, in the column's
/// colour, with the fold glyph flipped when the group is collapsed.
fn header(g: &Group, content_w: usize) -> ListItem<'static> {
    let (color, marker) = status_marker(g.status.as_deref().unwrap_or(""));
    let fold = if g.collapsed { G_FOLDED } else { G_OPEN };
    let count = g.count.to_string();
    let (name, name_w) = truncate(
        &g.name,
        content_w.saturating_sub(4 + count.len() + 2).max(4),
    );
    // fold(2) + marker(2) + name + gap + count
    let rule_w = content_w.saturating_sub(2 + 2 + name_w + 1 + count.len() + 1);
    let style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    let spans = vec![
        Span::styled(format!("{fold} "), Style::default().fg(color)),
        Span::styled(format!("{marker} "), Style::default().fg(color)),
        Span::styled(name, style),
        Span::styled(
            format!(" {} ", "─".repeat(rule_w)),
            Style::default().fg(NORD_SEL),
        ),
        Span::styled(count, Style::default().fg(color)),
    ];
    ListItem::new(Line::from(spans))
}

/// One card row: `● #365  Title ……  tag labels   Status`, with the status text
/// (and labels, when shown) flushed to the right edge and the title truncated to
/// fill the gap. `nest` is `Some(more_siblings_follow)` for a sub-issue rolled
/// up under the parent above it; `family` is `(done, total)` sub-issue counts
/// for a parent card, which also gets the parent glyph and a bold title.
fn card(
    it: &Item,
    content_w: usize,
    show_labels: bool,
    nest: Option<bool>,
    family: Option<(usize, usize)>,
) -> ListItem<'static> {
    let status = it.status.as_deref().unwrap_or("");
    let (scolor, marker) = status_marker(status);

    const MARKER_W: usize = 2; // glyph + space
    // "#365 " — a 4-wide field plus space, but a 5-digit number still fits.
    let num = format!("{:>4} ", it.number_label());
    let num_w = UnicodeWidthStr::width(num.as_str());
    let nest_w = if nest.is_some() { 2 } else { 0 }; // "├ " / "└ "
    let parent_w = if family.is_some() { 2 } else { 0 }; // glyph + space

    // Right cluster: sub-issue tally, optional labels, then the status word.
    let mut right: Vec<Span<'static>> = Vec::new();
    let mut right_w = 0usize;
    if let Some((done, total)) = family {
        let tally = format!("{done}/{total}");
        right_w += tally.len() + 2;
        right.push(Span::styled(
            format!("{tally}  "),
            Style::default().fg(if done == total { NORD_GREEN } else { NORD_CYAN }),
        ));
    }
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
    let left_w = nest_w + MARKER_W + num_w + parent_w;
    let title_avail = content_w.saturating_sub(left_w + right_w + 1);
    let (title_txt, title_w) = truncate(&it.title, title_avail);

    let gap = content_w.saturating_sub(left_w + title_w + right_w).max(1);

    let num_color = if it.number.is_some() {
        NORD_BLUE
    } else {
        NORD_MUTED
    };
    let title_style = if family.is_some() {
        Style::default().fg(NORD_TEXT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(NORD_TEXT)
    };

    let mut spans = Vec::new();
    match nest {
        Some(true) => spans.push(Span::styled(NEST_MID, Style::default().fg(NORD_MUTED))),
        Some(false) => spans.push(Span::styled(NEST_END, Style::default().fg(NORD_MUTED))),
        None => {}
    }
    spans.extend([
        Span::styled(format!("{marker} "), Style::default().fg(scolor)),
        Span::styled(num, Style::default().fg(num_color)),
    ]);
    if family.is_some() {
        spans.push(Span::styled(
            format!("{G_PARENT} "),
            Style::default().fg(NORD_CYAN),
        ));
    }
    spans.extend([
        Span::styled(title_txt, title_style),
        Span::raw(" ".repeat(gap)),
    ]);
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
    spans.extend(key("h/l", "group"));
    spans.push(sep());
    spans.extend(key("z", "fold"));
    spans.push(sep());
    spans.extend(key("/", "filter"));
    spans.push(sep());
    // Labels toggle carries its own on/off glyph so the state is readable at rest.
    spans.push(Span::styled("i ", Style::default().fg(NORD_CYAN)));
    spans.push(Span::styled(format!("{glyph} labels"), style));
    spans.push(sep());
    spans.extend(key("s", "start"));
    spans.push(sep());
    spans.extend(key("?", "help"));
    Line::from(spans)
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

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::config::schema::{Filter, Preset, ProjectConfig};
    use crate::model::{Item, ParentRef};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn card(number: u64, status: &str, labels: &[&str], parent: Option<u64>) -> Item {
        Item {
            id: format!("#{number}"),
            number: Some(number),
            title: format!("Ticket number {number} with a longish title"),
            repository: Some("o/r".into()),
            status: Some(status.into()),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            assignees: vec![],
            url: None,
            parent: parent.map(|n| ParentRef {
                repository: "o/r".into(),
                number: n,
            }),
        }
    }

    /// Render the grouped list into a test buffer and return its rows as text.
    fn draw(app: &mut App, width: u16) -> Vec<String> {
        let mut term = Terminal::new(TestBackend::new(width, 12)).unwrap();
        term.draw(|f| {
            let a = f.area();
            super::render(f, a, app);
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| match buf[(x, y)].symbol().chars().next() {
                        // Nerd Font glyphs (private use area) → visible stand-in.
                        Some(c) if ('\u{e000}'..='\u{f8ff}').contains(&c) => "#".to_string(),
                        _ => buf[(x, y)].symbol().to_string(),
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn grouped_list_renders_headers_badges_and_tree_lines() {
        let mut cfg = ProjectConfig::travel_smart();
        cfg.presets = vec![Preset {
            name: "frontend".into(),
            include: Filter {
                labels: vec!["Frontend".into()],
                ..Default::default()
            },
            include_parents: true,
        }];
        let items = vec![
            card(1002, "Ready To Implement", &["documentation"], None),
            card(1003, "In review", &["Frontend"], Some(1002)),
            card(1004, "In progress", &["Frontend"], Some(1002)),
            card(1010, "Ready To Implement", &["Frontend"], None),
        ];
        let mut app = App::new(items, cfg);
        let rows = draw(&mut app, 60);
        let text = rows.join("\n");
        assert!(text.contains("Ready To Implement"), "{text}");
        assert!(text.contains("In progress"), "{text}");
        assert!(
            text.contains("0/2"),
            "parent badge shows done/total: {text}"
        );
        assert!(
            text.contains("├"),
            "first child gets a mid connector: {text}"
        );
        assert!(
            text.contains("└"),
            "last child gets an end connector: {text}"
        );

        // Folding drops the cards but keeps the header, at any width.
        app.list_state.select(Some(0));
        app.toggle_group();
        for w in [12u16, 30, 60] {
            let rows = draw(&mut app, w);
            assert!(
                rows.iter().any(|r| r.contains('▸')),
                "folded glyph at width {w}"
            );
            assert!(
                !rows.iter().any(|r| r.contains("#1002")),
                "cards hidden at width {w}"
            );
        }
    }
}
