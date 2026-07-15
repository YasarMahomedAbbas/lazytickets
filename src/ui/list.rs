//! Left pane: the filtered task list.

use crate::app::App;
use crate::ui::{NORD_AMBER, NORD_CYAN, NORD_DIM};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    let rows: Vec<ListItem> = app
        .items
        .iter()
        .map(|it| {
            let status = it.status.as_deref().unwrap_or("");
            let line = Line::from(vec![
                Span::styled(format!("{:>5} ", it.number_label()), Style::default().fg(NORD_DIM)),
                Span::raw(it.title.clone()),
                Span::styled(format!("  {status}"), Style::default().fg(status_color(status))),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = format!(" {} — {} tickets ", app.board_label, app.items.len());
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

/// Amber for actively-worked columns, dim otherwise. A fuller status→colour map
/// arrives with per-project `status_order` in M3.
fn status_color(status: &str) -> ratatui::style::Color {
    if status.eq_ignore_ascii_case("In progress") {
        NORD_AMBER
    } else {
        NORD_DIM
    }
}
