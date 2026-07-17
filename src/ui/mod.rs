//! Rendering. Palette constants (Nord) live here so every pane shares them.

pub mod detail;
pub mod list;
pub mod modal;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Color;

// Nord palette (see PLAN.md §3 / about.md).
pub const NORD_CYAN: Color = Color::Rgb(0x88, 0xc0, 0xd0); // active/focused
pub const NORD_AMBER: Color = Color::Rgb(0xeb, 0xcb, 0x8b); // working
pub const NORD_DIM: Color = Color::DarkGray; // idle/secondary
pub const NORD_GREEN: Color = Color::Rgb(0xa3, 0xbe, 0x8c); // included/checked

pub fn render(frame: &mut Frame, app: &mut App) {
    // lazygit-style split: list left, detail right.
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .areas(frame.area());

    list::render(frame, left, app);
    detail::render(frame, right, app);

    if app.modal.is_open() {
        modal::render(frame, &app.modal);
    }
}
