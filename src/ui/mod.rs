//! Rendering. Palette constants (Nord) live here so every pane shares them.

pub mod list;

use crate::app::App;
use ratatui::Frame;
use ratatui::style::Color;

// Nord palette (see PLAN.md §3 / about.md).
pub const NORD_CYAN: Color = Color::Rgb(0x88, 0xc0, 0xd0); // active/focused
pub const NORD_AMBER: Color = Color::Rgb(0xeb, 0xcb, 0x8b); // working
pub const NORD_DIM: Color = Color::DarkGray; // idle/secondary

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    list::render(frame, area, app);
}
