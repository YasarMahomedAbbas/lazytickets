//! Left pane: preset tabs, the filtered task list, and the live-filter input.

use crate::app::{App, InputMode};
use crate::ui::{NORD_AMBER, NORD_CYAN, NORD_DIM};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    // tabs (1) · list (rest) · filter line (1, only while filtering or with a query)
    let show_filter = app.input_mode == InputMode::Filter || !app.filter_query.is_empty();
    let constraints = if show_filter {
        vec![Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]
    } else {
        vec![Constraint::Length(1), Constraint::Min(1)]
    };
    let chunks = Layout::vertical(constraints).split(area);

    render_tabs(frame, chunks[0], app);
    render_list(frame, chunks[1], app);
    if show_filter {
        render_filter(frame, chunks[2], app);
    }
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = Vec::new();
    for i in 0..app.preset_count() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(NORD_DIM)));
        }
        let style = if i == app.active_preset {
            Style::default().fg(NORD_CYAN).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(NORD_DIM)
        };
        spans.push(Span::styled(app.preset_name(i).to_string(), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let rows: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&i| {
            let it = &app.items[i];
            let status = it.status.as_deref().unwrap_or("");
            let line = Line::from(vec![
                Span::styled(format!("{:>5} ", it.number_label()), Style::default().fg(NORD_DIM)),
                Span::raw(it.title.clone()),
                Span::styled(format!("  {status}"), Style::default().fg(status_color(status))),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = format!(" {} — {} tickets ", app.board_label(), app.visible.len());
    let list = List::new(rows)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(NORD_CYAN)),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

fn render_filter(frame: &mut Frame, area: Rect, app: &App) {
    let active = app.input_mode == InputMode::Filter;
    let prompt = Span::styled("/", Style::default().fg(NORD_CYAN));
    let query = Span::raw(app.filter_query.clone());
    let cursor = if active {
        Span::styled("▏", Style::default().fg(NORD_CYAN))
    } else {
        Span::styled("  (Esc clears)", Style::default().fg(NORD_DIM))
    };
    frame.render_widget(Paragraph::new(Line::from(vec![prompt, query, cursor])), area);
}

/// Amber for actively-worked columns, dim otherwise.
fn status_color(status: &str) -> Color {
    if status.eq_ignore_ascii_case("In progress") {
        NORD_AMBER
    } else {
        NORD_DIM
    }
}
