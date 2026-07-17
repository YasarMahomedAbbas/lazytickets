//! Right pane: the selected ticket's description, comments, status and labels —
//! with inline images. The body is flattened into a list of physical rows (text
//! lines and image blocks), which lets us scroll the whole thing as one document
//! and place each image exactly where it appears in the text.

use crate::app::{App, DetailState};
use crate::attach::ContentPart;
use crate::gh::issue::IssueDetail;
use crate::images::{ImageEntry, Images};
use crate::ui::{NORD_CYAN, NORD_GREEN, NORD_MUTED, NORD_PURPLE, NORD_TEXT};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui_image::Image;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// Nerd Font glyphs shared with the list vocabulary.
const G_TAG: &str = "\u{f02c}"; // tags
const G_LINK: &str = "\u{f0c1}"; // link — issue url
const G_COMMENT: &str = "\u{f075}"; // speech bubble — comments
const G_USER: &str = "\u{f007}"; // person — comment author
const G_IMAGE: &str = "\u{f03e}"; // picture — image placeholder

/// Absolute cap on an inline image's height, so it never dominates a tall pane.
const MAX_IMAGE_ROWS: u16 = 40;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    let has_images = app
        .detail_parts
        .iter()
        .any(|p| matches!(p, ContentPart::Image(_)));
    let hint = if has_images && app.images.enabled() {
        " detail · J/K scroll "
    } else {
        " detail "
    };
    let block = Block::default()
        .title(Span::styled(hint, Style::default().fg(NORD_MUTED)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NORD_MUTED));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    // Wipe the content region every frame so a scrolled-out Kitty image (anchored
    // to now-blank cells) doesn't linger.
    frame.render_widget(Clear, inner);

    if matches!(app.detail, DetailState::Loaded(_)) {
        render_loaded(frame, inner, app);
        return;
    }
    let msg = match &app.detail {
        DetailState::Empty => "No ticket selected.",
        DetailState::Draft => "Draft item — no issue to display.",
        DetailState::Loading => "Loading…",
        DetailState::Error(e) => return placeholder(frame, inner, &format!("Error: {e}")),
        DetailState::Loaded(_) => unreachable!(),
    };
    placeholder(frame, inner, msg);
}

fn placeholder(frame: &mut Frame, area: Rect, msg: &str) {
    let p = Paragraph::new(msg).style(Style::default().fg(NORD_MUTED));
    frame.render_widget(p, area);
}

/// A single physical row of the flattened detail document.
enum Row {
    Line(Line<'static>),
    Image { url: String, rows: u16, state: ImgKind },
}

enum ImgKind {
    Ready,
    Loading,
    Failed(String),
}

impl Row {
    fn height(&self) -> u16 {
        match self {
            Row::Line(_) => 1,
            Row::Image { rows, .. } => *rows,
        }
    }
}

fn render_loaded(frame: &mut Frame, inner: Rect, app: &mut App) {
    let App {
        detail,
        detail_parts,
        detail_scroll,
        images,
        ..
    } = app;
    let DetailState::Loaded(d) = &*detail else {
        return;
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = build_rows(d, detail_parts, images, inner.width, inner.height);

    // Clamp scroll to the content height now that we know it.
    let total: u16 = rows.iter().map(Row::height).sum();
    let max_scroll = total.saturating_sub(inner.height);
    *detail_scroll = (*detail_scroll).min(max_scroll);
    let scroll = *detail_scroll;

    let mut y = 0u16; // document row of the current block's top
    for row in &rows {
        let h = row.height();
        let top = y;
        let bottom = y + h;
        y = bottom;

        // Fully outside the viewport.
        if bottom <= scroll || top >= scroll + inner.height {
            continue;
        }

        match row {
            Row::Line(line) => {
                let screen_y = inner.y + (top - scroll);
                let rect = Rect::new(inner.x, screen_y, inner.width, 1);
                frame.render_widget(Paragraph::new(line.clone()), rect);
            }
            Row::Image { url, rows: irows, state } => {
                // Draw only once the image's top edge is in view; the pane is at
                // least as tall as any image (rows are capped to inner.height), so
                // an image is fully visible somewhere in its scroll range and then
                // scrolls cleanly off the top.
                if top < scroll {
                    continue;
                }
                let screen_y = inner.y + (top - scroll);
                let avail = (scroll + inner.height) - top;
                let draw_h = (*irows).min(avail);
                let rect = Rect::new(inner.x, screen_y, inner.width, draw_h);
                match state {
                    ImgKind::Ready => {
                        if let Some(proto) = images.protocol_for(url, inner.width, *irows) {
                            frame.render_widget(Image::new(proto).allow_clipping(true), rect);
                        } else {
                            note(frame, rect, G_IMAGE, "image couldn't be displayed", NORD_MUTED);
                        }
                    }
                    ImgKind::Loading => note(frame, rect, G_IMAGE, "loading image…", NORD_MUTED),
                    ImgKind::Failed(e) => {
                        note(frame, rect, "\u{f071}", &format!("image failed: {e}"), NORD_PURPLE)
                    }
                }
            }
        }
    }
}

/// A one-line labelled note drawn at the top of an image's reserved area (used
/// for loading / failed / unsupported states).
fn note(frame: &mut Frame, area: Rect, glyph: &str, msg: &str, color: Color) {
    let rect = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{glyph}  "), Style::default().fg(color)),
            Span::styled(msg.to_string(), Style::default().fg(color)),
        ])),
        rect,
    );
}

/// Flatten header + body parts + comments into physical rows at `width`.
fn build_rows(
    d: &IssueDetail,
    parts: &[ContentPart],
    images: &Images,
    width: u16,
    height: u16,
) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    let text = Style::default().fg(NORD_TEXT);

    // --- header ---
    for l in wrap(&d.title, width, Style::default().fg(NORD_CYAN).add_modifier(Modifier::BOLD)) {
        rows.push(Row::Line(l));
    }
    rows.push(Row::Line(state_line(d)));
    rows.push(Row::Line(Line::from(Span::styled(
        format!("{G_LINK} {}", d.url),
        Style::default().fg(NORD_MUTED),
    ))));
    rows.push(Row::Line(Line::from("")));

    // --- body (text + inline images) ---
    let max_rows = height.min(MAX_IMAGE_ROWS);
    for part in parts {
        match part {
            ContentPart::Text(t) => {
                for src in t.split('\n') {
                    for l in wrap(src, width, text) {
                        rows.push(Row::Line(l));
                    }
                }
            }
            ContentPart::Image(iref) => {
                let (state, dims) = match images.cache.get(&iref.url) {
                    Some(ImageEntry::Ready { img, .. }) => (ImgKind::Ready, (img.width(), img.height())),
                    Some(ImageEntry::Failed(e)) => (ImgKind::Failed(e.clone()), (0, 0)),
                    _ => (
                        ImgKind::Loading,
                        (iref.width.unwrap_or(4), iref.height.unwrap_or(3)),
                    ),
                };
                let irows = match state {
                    ImgKind::Failed(_) => 1,
                    _ => images.rows_for(dims.0, dims.1, width, max_rows),
                };
                rows.push(Row::Image {
                    url: iref.url.clone(),
                    rows: irows,
                    state,
                });
            }
        }
    }

    // --- comments ---
    if !d.comments.is_empty() {
        rows.push(Row::Line(Line::from("")));
        rows.push(Row::Line(Line::from(Span::styled(
            format!("{G_COMMENT}  {} comments", d.comments.len()),
            Style::default().fg(NORD_MUTED).add_modifier(Modifier::BOLD),
        ))));
        for c in &d.comments {
            rows.push(Row::Line(Line::from("")));
            rows.push(Row::Line(Line::from(Span::styled(
                format!("{G_USER} {}", c.author),
                Style::default().fg(NORD_CYAN),
            ))));
            for src in c.body.split('\n') {
                for l in wrap(src, width, text) {
                    rows.push(Row::Line(l));
                }
            }
        }
    }

    rows
}

/// The `state · labels` header line: an open/closed pill plus any labels.
fn state_line(d: &IssueDetail) -> Line<'static> {
    let open = d.state.eq_ignore_ascii_case("open");
    let pill_bg = if open { NORD_GREEN } else { NORD_PURPLE };
    let mut spans = vec![Span::styled(
        format!(" {} ", d.state),
        Style::default()
            .bg(pill_bg)
            .fg(Color::Rgb(0x2e, 0x34, 0x40))
            .add_modifier(Modifier::BOLD),
    )];
    if !d.labels.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("{G_TAG} {}", d.labels.join(", ")),
            Style::default().fg(NORD_PURPLE),
        ));
    }
    Line::from(spans)
}

/// Greedy word-wrap of one source line (no embedded newlines) to `width` columns,
/// applying `style` to every physical line produced. An empty input yields one
/// blank line so paragraph spacing is preserved.
fn wrap(text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    let w = width.max(1) as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;

    let flush = |cur: &mut String, cur_w: &mut usize, lines: &mut Vec<Line<'static>>| {
        lines.push(Line::from(Span::styled(std::mem::take(cur), style)));
        *cur_w = 0;
    };

    for word in text.split(' ') {
        if word.is_empty() {
            // Collapse runs of spaces to a single separating space.
            if cur_w < w {
                cur.push(' ');
                cur_w += 1;
            }
            continue;
        }
        let ww = UnicodeWidthStr::width(word);
        let need = if cur.is_empty() { ww } else { ww + 1 };
        if cur_w + need > w && !cur.is_empty() {
            flush(&mut cur, &mut cur_w, &mut lines);
        }
        if ww > w {
            if !cur.is_empty() {
                flush(&mut cur, &mut cur_w, &mut lines);
            }
            for chunk in hard_split(word, w) {
                lines.push(Line::from(Span::styled(chunk, style)));
            }
            continue;
        }
        if !cur.is_empty() {
            cur.push(' ');
            cur_w += 1;
        }
        cur.push_str(word);
        cur_w += ww;
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(Line::from(Span::styled(cur, style)));
    }
    lines
}

/// Break a word longer than `w` columns into `w`-wide chunks.
fn hard_split(word: &str, w: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    for ch in word.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if cur_w + cw > w && !cur.is_empty() {
            chunks.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push(ch);
        cur_w += cw;
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_on_width_and_keeps_blank_lines() {
        let out = wrap("the quick brown fox", 9, Style::default());
        let text: Vec<String> = out
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
            .collect();
        assert_eq!(text, vec!["the quick".to_string(), "brown fox".to_string()]);

        // Empty input still occupies one row (paragraph spacing).
        assert_eq!(wrap("", 10, Style::default()).len(), 1);
    }

    #[test]
    fn wrap_hard_splits_overlong_words() {
        let out = wrap("supercalifragilistic", 5, Style::default());
        assert!(out.len() >= 4, "a 20-char word wraps into 5-col chunks");
        for l in &out {
            let w: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
            assert!(w <= 5);
        }
    }

    /// End-to-end layout guard: a loaded detail with an inline image must render
    /// (and survive an over-scroll) without panicking on the row/scroll math.
    #[test]
    fn renders_inline_image_and_survives_overscroll() {
        use crate::app::App;
        use crate::config::schema::ProjectConfig;
        use crate::images::{ImageEntry, Images};
        use crate::model::Item;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let item = Item {
            id: "a".into(),
            number: Some(1),
            title: "t".into(),
            repository: Some("o/r".into()),
            status: Some("Refine".into()),
            labels: vec![],
            assignees: vec![],
            url: None,
        };
        let mut app = App::new(vec![item], ProjectConfig::travel_smart());
        // Half-blocks needs no terminal query, so it's safe under test.
        app.images = Images::new(Some(ratatui_image::picker::Picker::halfblocks()));
        let url = "https://x/y.png".to_string();
        app.images.cache.insert(
            url.clone(),
            ImageEntry::Ready {
                img: image::DynamicImage::new_rgb8(8, 8),
                proto: None,
                cols: 0,
                rows: 0,
            },
        );
        app.show_detail(IssueDetail {
            title: "A ticket with a picture".into(),
            body: format!("before the image\n![shot]({url})\nafter the image"),
            state: "OPEN".into(),
            labels: vec!["bug".into()],
            url: "https://example/issues/1".into(),
            comments: vec![],
        });

        let mut term = Terminal::new(TestBackend::new(40, 20)).unwrap();
        term.draw(|f| {
            let a = f.area();
            super::render(f, a, &mut app);
        })
        .unwrap();

        // Scroll far past the end; render must clamp and not panic.
        app.scroll_detail(500);
        term.draw(|f| {
            let a = f.area();
            super::render(f, a, &mut app);
        })
        .unwrap();
    }
}
