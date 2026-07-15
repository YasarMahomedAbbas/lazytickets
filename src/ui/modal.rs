//! Centered overlay for the start-work flow (confirm prompt + messages).

use crate::app::Modal;
use crate::ui::{NORD_AMBER, NORD_CYAN};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub fn render(frame: &mut Frame, modal: &Modal) {
    // The status mover renders its own selectable list; the rest are text popups.
    if let Modal::StatusMove { options, selected, .. } = modal {
        render_mover(frame, options, *selected);
        return;
    }
    if matches!(modal, Modal::Help) {
        render_help(frame);
        return;
    }

    let (title, body, border) = match modal {
        Modal::Confirm { issue, skill, session, .. } => (
            " Start work ",
            format!(
                "Start #{issue} with the '{skill}' skill\nin {session}:claude?\n\nThis clears the Claude pane first.\n\n[y] start    [n] cancel"
            ),
            NORD_CYAN,
        ),
        Modal::Message(msg) => (" lazytickets ", format!("{msg}\n\n[any key] dismiss"), NORD_AMBER),
        Modal::StatusMove { .. } | Modal::Help | Modal::None => return,
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
                Line::styled(format!("▶ {name}"), Style::default().add_modifier(Modifier::REVERSED))
            } else {
                Line::raw(format!("  {name}"))
            }
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled("j/k move · Enter set · Esc cancel", Style::default().fg(NORD_AMBER)));

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

fn render_help(frame: &mut Frame) {
    let rows = [
        ("j / k · ↓ / ↑", "move selection"),
        ("Tab / S-Tab · 1-9", "switch preset tab"),
        ("/", "live fuzzy filter (Esc clears)"),
        ("s", "start work (drive claude pane)"),
        ("m", "move status column"),
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
    lines.push(Line::styled("[any key] dismiss", Style::default().fg(NORD_AMBER)));

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
