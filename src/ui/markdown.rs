//! Markdown → styled `Line`s for the detail pane. Issue bodies are GitHub
//! Markdown; rendering headings, emphasis, code, lists, quotes and tables as
//! terminal styling (rather than showing the raw `**` and `#`) makes a ticket
//! read the way it does on github.com. Word-wrapped to the pane width here,
//! with the list bullet / quote bar carried onto continuation lines.

use crate::ui::{NORD_AMBER, NORD_BLUE, NORD_CYAN, NORD_MUTED, NORD_SEL, NORD_TEXT};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const BULLET: &str = "\u{2022}"; // •
const QUOTE_BAR: &str = "\u{258e} "; // ▎
const TASK_DONE: &str = "\u{f14a}"; // checked box (Nerd Font)
const TASK_TODO: &str = "\u{f096}"; // empty box

/// A styled run of inline text, before wrapping.
type Frag = (String, Style);

/// Render `src` as Markdown into physical lines at most `width` columns wide.
pub fn render(src: &str, width: u16) -> Vec<Line<'static>> {
    let mut r = Renderer::new(width.max(1) as usize);
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    for ev in Parser::new_ext(src, opts) {
        r.event(ev);
    }
    r.finish()
}

struct Renderer {
    width: usize,
    lines: Vec<Line<'static>>,
    /// Inline text of the block being built.
    buf: Vec<Frag>,
    /// Inline style stack (`**`, `_`, links…); the top applies to new text.
    styles: Vec<Style>,
    /// Open lists: `Some(next number)` for ordered, `None` for bullets.
    lists: Vec<Option<u64>>,
    /// Bullet awaiting the first line of the current list item.
    bullet: Option<String>,
    quote_depth: usize,
    /// Inside a fenced/indented code block: text is collected verbatim.
    code: Option<String>,
    /// Table state: cells of the current row, and whether it's the header row.
    table_row: Option<Vec<String>>,
    table_head: bool,
}

impl Renderer {
    fn new(width: usize) -> Self {
        Self {
            width,
            lines: Vec::new(),
            buf: Vec::new(),
            styles: vec![Style::default().fg(NORD_TEXT)],
            lists: Vec::new(),
            bullet: None,
            quote_depth: 0,
            code: None,
            table_row: None,
            table_head: false,
        }
    }

    fn style(&self) -> Style {
        *self.styles.last().expect("base style is never popped")
    }

    fn push_style(&mut self, f: impl FnOnce(Style) -> Style) {
        self.styles.push(f(self.style()));
    }

    fn pop_style(&mut self) {
        if self.styles.len() > 1 {
            self.styles.pop();
        }
    }

    fn text(&mut self, s: &str, style: Style) {
        if let Some(last) = self.buf.last_mut()
            && last.1 == style
        {
            last.0.push_str(s);
        } else {
            self.buf.push((s.to_string(), style));
        }
    }

    /// Indentation for the current nesting: quote bars, then list depth.
    fn prefix(&self) -> (String, String) {
        let quote = QUOTE_BAR.repeat(self.quote_depth);
        let indent = "  ".repeat(self.lists.len().saturating_sub(1));
        let rest = if self.lists.is_empty() {
            String::new()
        } else {
            "  ".to_string()
        };
        (format!("{quote}{indent}"), format!("{quote}{indent}{rest}"))
    }

    /// Emit the pending inline block as wrapped lines. A list item's bullet
    /// leads the first line; continuation lines hang under the text.
    fn flush(&mut self) {
        if self.buf.is_empty() && self.bullet.is_none() {
            return;
        }
        let frags = std::mem::take(&mut self.buf);
        let (base, hang) = self.prefix();
        let in_list = !self.lists.is_empty();
        let first = match self.bullet.take() {
            Some(b) => format!("{base}{b}"),
            None if in_list => hang.clone(),
            None => base.clone(),
        };
        let rest = if in_list { hang } else { base };
        let prefix_style = Style::default().fg(NORD_MUTED);
        let wrapped = wrap_frags(&frags, self.width, &first, &rest, prefix_style);
        self.lines.extend(wrapped);
    }

    /// One blank separator line, never doubled.
    fn blank(&mut self) {
        let last_blank = self
            .lines
            .last()
            .is_some_and(|l| l.spans.iter().all(|s| s.content.trim().is_empty()));
        if !self.lines.is_empty() && !last_blank {
            self.lines.push(Line::from(""));
        }
    }

    fn event(&mut self, ev: Event<'_>) {
        // A code block swallows everything until its end.
        if let Some(code) = &mut self.code {
            match ev {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => self.end_code(),
                _ => {}
            }
            return;
        }
        // Table cells are collected as plain text.
        if let Some(row) = &mut self.table_row {
            match ev {
                Event::Text(t) | Event::Code(t) => {
                    if let Some(cell) = row.last_mut() {
                        cell.push_str(&t);
                    }
                }
                Event::Start(Tag::TableCell) => row.push(String::new()),
                Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => self.end_row(),
                Event::SoftBreak | Event::HardBreak => {
                    if let Some(cell) = row.last_mut() {
                        cell.push(' ');
                    }
                }
                _ => {}
            }
            return;
        }

        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t, self.style()),
            Event::Code(t) => self.text(
                &format!(" {t} "),
                Style::default().fg(NORD_AMBER).bg(NORD_SEL),
            ),
            Event::Html(h) | Event::InlineHtml(h) => {
                self.text(h.trim_end_matches('\n'), Style::default().fg(NORD_MUTED))
            }
            Event::SoftBreak => self.text(" ", self.style()),
            Event::HardBreak => {
                // Break the paragraph here but stay in the same block.
                self.flush();
                self.bullet = None;
            }
            Event::Rule => {
                self.flush();
                self.blank();
                self.lines.push(Line::from(Span::styled(
                    "─".repeat(self.width.min(40)),
                    Style::default().fg(NORD_MUTED),
                )));
                self.blank();
            }
            Event::TaskListMarker(done) => {
                let glyph = if done { TASK_DONE } else { TASK_TODO };
                // Replace the bullet with the checkbox glyph.
                self.bullet = Some(format!("{glyph} "));
            }
            Event::FootnoteReference(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush();
                self.blank();
                let style = match level {
                    HeadingLevel::H1 => Style::default()
                        .fg(NORD_CYAN)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                    HeadingLevel::H2 => Style::default().fg(NORD_CYAN).add_modifier(Modifier::BOLD),
                    _ => Style::default().fg(NORD_TEXT).add_modifier(Modifier::BOLD),
                };
                self.styles.push(style);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote_depth += 1;
                self.push_style(|s| s.fg(NORD_MUTED));
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.code = Some(String::new());
            }
            Tag::List(start) => {
                self.flush();
                if self.lists.is_empty() {
                    self.blank();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let bullet = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let b = format!("{n}. ");
                        *n += 1;
                        b
                    }
                    _ => format!("{BULLET} "),
                };
                self.bullet = Some(bullet);
            }
            Tag::Emphasis => self.push_style(|s| s.add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(|s| s.add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(|s| s.add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { .. } => {
                self.push_style(|s| s.fg(NORD_BLUE).add_modifier(Modifier::UNDERLINED))
            }
            Tag::Image { dest_url, .. } => {
                // Images are normally split out before we get here; show any
                // that slip through as a muted reference.
                self.text(
                    &format!("[image: {dest_url}]"),
                    Style::default().fg(NORD_MUTED),
                );
                self.push_style(|s| s.fg(NORD_MUTED));
            }
            Tag::Table(_) => {
                self.flush();
                self.blank();
            }
            Tag::TableHead => {
                self.table_head = true;
                self.table_row = Some(Vec::new());
            }
            Tag::TableRow => {
                self.table_head = false;
                self.table_row = Some(Vec::new());
            }
            Tag::TableCell
            | Tag::HtmlBlock
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::MetadataBlock(_)
            | Tag::Superscript
            | Tag::Subscript => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Heading(_) => {
                self.flush();
                self.pop_style();
                self.blank();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.pop_style();
                if self.quote_depth == 0 {
                    self.blank();
                }
            }
            TagEnd::Item => {
                self.flush();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Link
            | TagEnd::Image => self.pop_style(),
            TagEnd::Table => self.blank(),
            TagEnd::HtmlBlock => {
                self.flush();
                self.blank();
            }
            _ => {}
        }
    }

    fn end_code(&mut self) {
        let Some(code) = self.code.take() else {
            return;
        };
        let (base, _) = self.prefix();
        let style = Style::default().fg(NORD_AMBER);
        let indent = format!("{base}  ");
        let avail = self
            .width
            .saturating_sub(UnicodeWidthStr::width(indent.as_str()))
            .max(1);
        self.blank();
        for raw in code.trim_end_matches('\n').split('\n') {
            let src = raw.replace('\t', "    ");
            let chunks = if UnicodeWidthStr::width(src.as_str()) <= avail {
                vec![src]
            } else {
                hard_split(&src, avail)
            };
            for chunk in chunks {
                self.lines.push(Line::from(vec![
                    Span::styled(indent.clone(), Style::default().fg(NORD_MUTED)),
                    Span::styled(chunk, style),
                ]));
            }
        }
        self.blank();
    }

    fn end_row(&mut self) {
        let Some(cells) = self.table_row.take() else {
            return;
        };
        let style = if self.table_head {
            Style::default().fg(NORD_CYAN).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(NORD_TEXT)
        };
        let (base, _) = self.prefix();
        let mut spans = vec![Span::styled(base, Style::default().fg(NORD_MUTED))];
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" │ ", Style::default().fg(NORD_MUTED)));
            }
            spans.push(Span::styled(cell.trim().to_string(), style));
        }
        self.lines.push(Line::from(spans));
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush();
        if self.code.is_some() {
            self.end_code();
        }
        // Trim the trailing separator so the caller controls spacing.
        while self
            .lines
            .last()
            .is_some_and(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
        {
            self.lines.pop();
        }
        self.lines
    }
}

/// Greedy word-wrap of styled fragments to `width` columns. `first` prefixes the
/// first line and `rest` every continuation line, both drawn in `prefix_style`.
fn wrap_frags(
    frags: &[Frag],
    width: usize,
    first: &str,
    rest: &str,
    prefix_style: Style,
) -> Vec<Line<'static>> {
    let mut w = Wrapper {
        lines: Vec::new(),
        cur: Vec::new(),
        cur_w: 0,
        prefix: first.to_string(),
        rest: rest.to_string(),
        prefix_style,
        width,
    };
    for (text, style) in frags {
        let mut pending_space = text.starts_with(' ');
        for word in text.split(' ') {
            if word.is_empty() {
                continue;
            }
            w.word(word, *style, pending_space);
            pending_space = true;
        }
        // A fragment ending in a space separates it from the next fragment.
        if text.ends_with(' ') && w.cur_w > 0 {
            w.cur.push(Span::styled(" ", *style));
            w.cur_w += 1;
        }
    }
    if w.cur_w > 0 || w.lines.is_empty() {
        w.flush();
    }
    w.lines
}

/// Line-packing state for `wrap_frags`.
struct Wrapper {
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    cur_w: usize,
    /// Prefix for the line being built: `first` until it's emitted, then `rest`.
    prefix: String,
    rest: String,
    prefix_style: Style,
    width: usize,
}

impl Wrapper {
    fn avail(&self) -> usize {
        self.width
            .saturating_sub(UnicodeWidthStr::width(self.prefix.as_str()))
            .max(1)
    }

    fn flush(&mut self) {
        let mut spans = vec![Span::styled(self.prefix.clone(), self.prefix_style)];
        spans.append(&mut self.cur);
        self.lines.push(Line::from(spans));
        self.cur_w = 0;
        self.prefix = self.rest.clone();
    }

    /// Place one word, breaking the line first if it doesn't fit and hard-splitting
    /// a word wider than a whole line.
    fn word(&mut self, word: &str, style: Style, space_before: bool) {
        let ww = UnicodeWidthStr::width(word);
        let sep = usize::from(self.cur_w > 0 && space_before);
        if self.cur_w > 0 && self.cur_w + sep + ww > self.avail() {
            self.flush();
        }
        let avail = self.avail();
        if ww > avail {
            if self.cur_w > 0 {
                self.flush();
            }
            let chunks = hard_split(word, avail);
            let n = chunks.len();
            for (i, chunk) in chunks.into_iter().enumerate() {
                self.cur_w = UnicodeWidthStr::width(chunk.as_str());
                self.cur.push(Span::styled(chunk, style));
                if i + 1 < n {
                    self.flush();
                }
            }
            return;
        }
        if self.cur_w > 0 && space_before {
            self.cur.push(Span::styled(" ", style));
            self.cur_w += 1;
        }
        self.cur.push(Span::styled(word.to_string(), style));
        self.cur_w += ww;
    }
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

    fn text_of(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn headings_lists_and_emphasis_render_as_styled_lines() {
        let src = "# Title\n\nSome **bold** and _it_ text.\n\n- one\n- two\n  continued\n\n1. first\n2. second\n";
        let lines = render(src, 40);
        let t = text_of(&lines);
        assert_eq!(t[0], "Title");
        assert!(
            lines[0].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(
            t[2], "Some bold and it text.",
            "markup stripped, words spaced"
        );
        assert!(t.iter().any(|l| l == "• one"));
        assert!(t.iter().any(|l| l == "• two continued"), "soft break joins");
        assert!(t.iter().any(|l| l == "1. first"));
        assert!(t.iter().any(|l| l == "2. second"));
    }

    #[test]
    fn list_continuation_lines_hang_under_the_text() {
        let lines = render("- alpha beta gamma delta", 13);
        let t = text_of(&lines);
        assert_eq!(t, vec!["• alpha beta", "  gamma delta"]);
    }

    #[test]
    fn code_blocks_keep_whitespace_and_tasks_get_checkboxes() {
        let src = "```\nfn x() {\n    1\n}\n```\n\n- [x] done\n- [ ] todo\n";
        let t = text_of(&render(src, 40));
        assert!(
            t.iter().any(|l| l == "      1"),
            "4-space indent kept: {t:?}"
        );
        assert!(
            t.iter()
                .any(|l| l.starts_with(TASK_DONE) && l.ends_with("done"))
        );
        assert!(
            t.iter()
                .any(|l| l.starts_with(TASK_TODO) && l.ends_with("todo"))
        );
    }

    #[test]
    fn blockquotes_carry_the_bar_and_tables_join_cells() {
        let src = "> quoted words\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let t = text_of(&render(src, 40));
        assert_eq!(t[0], "▎ quoted words");
        assert!(t.iter().any(|l| l == "a │ b"));
        assert!(t.iter().any(|l| l == "1 │ 2"));
    }

    #[test]
    fn wraps_to_width_and_never_overflows() {
        let src = "the quick brown fox jumps over the lazy dog again and again";
        for l in render(src, 10) {
            let w: usize = l
                .spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            assert!(w <= 10, "line too wide: {:?}", l);
        }
    }
}
