//! Right pane: the selected ticket's description, comments, status and labels.

use crate::app::{App, DetailState};
use crate::gh::issue::IssueDetail;
use crate::ui::{NORD_AMBER, NORD_CYAN, NORD_DIM};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" detail ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NORD_DIM));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    match &app.detail {
        DetailState::Empty => placeholder(frame, inner, "No ticket selected."),
        DetailState::Draft => placeholder(frame, inner, "Draft item — no issue to display."),
        DetailState::Loading => placeholder(frame, inner, "Loading…"),
        DetailState::Error(e) => placeholder(frame, inner, &format!("Error: {e}")),
        DetailState::Loaded(d) => loaded(frame, inner, d),
    }
}

fn placeholder(frame: &mut Frame, area: Rect, msg: &str) {
    let p = Paragraph::new(msg).style(Style::default().fg(NORD_DIM));
    frame.render_widget(p, area);
}

fn loaded(frame: &mut Frame, area: Rect, d: &IssueDetail) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        d.title.clone(),
        Style::default().fg(NORD_CYAN).add_modifier(Modifier::BOLD),
    )));

    // state · labels
    let mut meta = vec![Span::styled(
        d.state.clone(),
        Style::default().fg(NORD_AMBER),
    )];
    if !d.labels.is_empty() {
        meta.push(Span::styled("  ", Style::default()));
        meta.push(Span::styled(
            d.labels.join(", "),
            Style::default().fg(NORD_DIM),
        ));
    }
    lines.push(Line::from(meta));
    lines.push(Line::from(Span::styled(
        d.url.clone(),
        Style::default().fg(NORD_DIM),
    )));
    lines.push(Line::from(""));

    // body
    for l in d.body.lines() {
        lines.push(Line::from(l.to_string()));
    }

    // comments
    if !d.comments.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("── {} comment(s) ──", d.comments.len()),
            Style::default().fg(NORD_DIM),
        )));
        for c in &d.comments {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("@{}", c.author),
                Style::default().fg(NORD_CYAN),
            )));
            for l in c.body.lines() {
                lines.push(Line::from(l.to_string()));
            }
        }
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}
