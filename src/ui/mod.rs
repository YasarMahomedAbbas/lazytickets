//! Rendering. Palette constants (Nord) live here so every pane shares them.

pub mod detail;
pub mod list;
pub mod markdown;
pub mod modal;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Color;

// Nord palette (see PLAN.md §3 / about.md).
pub const NORD_CYAN: Color = Color::Rgb(0x88, 0xc0, 0xd0); // frost8 — active/focused
pub const NORD_AMBER: Color = Color::Rgb(0xeb, 0xcb, 0x8b); // aurora13 — working
pub const NORD_DIM: Color = Color::DarkGray; // idle/secondary (modals, borders)
pub const NORD_GREEN: Color = Color::Rgb(0xa3, 0xbe, 0x8c); // aurora14 — done/checked
pub const NORD_RED: Color = Color::Rgb(0xbf, 0x61, 0x6a); // aurora11 — blocked
pub const NORD_PURPLE: Color = Color::Rgb(0xb4, 0x8e, 0xad); // aurora15 — review / labels
pub const NORD_BLUE: Color = Color::Rgb(0x81, 0xa1, 0xc1); // frost9 — not-started / numbers
// A readable secondary tone to replace DarkGray for *text* on the main screen —
// legible on a dark background where `NORD_DIM` (DarkGray) all but vanishes.
pub const NORD_TEXT: Color = Color::Rgb(0xe5, 0xe9, 0xf0); // snow5 — primary text
pub const NORD_MUTED: Color = Color::Rgb(0x7c, 0x88, 0xa0); // slate — secondary text
pub const NORD_SEL: Color = Color::Rgb(0x43, 0x4c, 0x5e); // polar2 — selection bar

// Status markers: shape encodes progress, colour encodes which column.
const M_TODO: &str = "\u{f10c}"; // hollow circle — not started
const M_ACTIVE: &str = "\u{f111}"; // filled circle — in flight
const M_DONE: &str = "\u{f058}"; // check-circle — done
const M_BLOCKED: &str = "\u{f057}"; // times-circle — blocked

/// Marker glyph + colour for a status column. Shape encodes progress (hollow →
/// filled → check), colour encodes the kind, so a board's columns read at a glance
/// regardless of the exact status names a project uses.
pub fn status_marker(status: &str) -> (Color, &'static str) {
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
