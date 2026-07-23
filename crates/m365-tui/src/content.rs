//! Turning Graph message/mail bodies into styled terminal text.
//!
//! Both Outlook mail and Teams messages arrive as HTML (or occasionally plain
//! text). We convert HTML → Markdown once (cached by the caller), then render
//! that Markdown into a styled ratatui [`Text`] here. Using our own
//! `pulldown-cmark` walk (rather than the `tui-markdown` crate) keeps us off its
//! ratatui-0.30 requirement and gives full control over the styling.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const CODE_FG: Color = Color::Rgb(220, 185, 130);
const CODE_BG: Color = Color::Rgb(40, 40, 52);

/// Convert a Graph body to Markdown. HTML is converted with `htmd`; anything
/// else (plain text) is passed through verbatim so stray `*`/`_` aren't
/// misread as emphasis.
pub fn body_to_markdown(content_type: Option<&str>, raw: &str) -> String {
    let is_html = content_type
        .map(|c| c.eq_ignore_ascii_case("html"))
        .unwrap_or(false);
    if is_html {
        htmd::convert(raw).unwrap_or_else(|_| strip_html(raw))
    } else {
        raw.to_string()
    }
}

/// Render Markdown into styled terminal text.
pub fn markdown_to_text(md: &str) -> Text<'static> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);

    let mut r = Renderer::default();
    for ev in Parser::new_ext(md, opts) {
        r.event(ev);
    }
    r.finish()
}

#[derive(Default)]
struct Renderer {
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    /// Inline style stack (top = current); pushed by strong/emphasis/link/…
    style_stack: Vec<Style>,
    /// List nesting; `Some(n)` = ordered with next number `n`, `None` = bullet.
    list_stack: Vec<Option<u64>>,
    in_code_block: bool,
    blockquote: usize,
    link_url: Option<String>,
}

impl Renderer {
    fn cur_style(&self) -> Style {
        self.style_stack.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, f: impl FnOnce(Style) -> Style) {
        let base = self.cur_style();
        self.style_stack.push(f(base));
    }

    fn line_has_content(&self) -> bool {
        self.spans.iter().any(|s| !s.content.trim().is_empty())
    }

    fn flush_line(&mut self) {
        let spans = std::mem::take(&mut self.spans);
        self.lines.push(Line::from(spans));
    }

    /// Add a blank separator unless the previous line is already blank.
    fn ensure_blank(&mut self) {
        if let Some(last) = self.lines.last() {
            let blank = last.spans.iter().all(|s| s.content.trim().is_empty());
            if !blank {
                self.lines.push(Line::from(""));
            }
        }
    }

    /// Begin a line with blockquote + list-depth indentation.
    fn start_block_line(&mut self) {
        let mut prefix = String::new();
        for _ in 0..self.blockquote {
            prefix.push_str("▏ ");
        }
        let depth = self.list_stack.len().saturating_sub(1);
        for _ in 0..depth {
            prefix.push_str("  ");
        }
        if !prefix.is_empty() {
            self.spans.push(Span::styled(prefix, Style::default().fg(DIM)));
        }
    }

    fn event(&mut self, ev: Event) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if self.in_code_block {
                    let text = t.to_string();
                    let mut parts = text.split('\n').peekable();
                    while let Some(part) = parts.next() {
                        self.spans
                            .push(Span::styled(part.to_string(), Style::default().fg(CODE_FG)));
                        if parts.peek().is_some() {
                            self.flush_line();
                            self.start_block_line();
                            self.spans.push(Span::raw("    "));
                        }
                    }
                } else {
                    let style = self.cur_style();
                    self.spans.push(Span::styled(t.to_string(), style));
                }
            }
            Event::Code(t) => {
                self.spans.push(Span::styled(
                    format!(" {t} "),
                    Style::default().fg(CODE_FG).bg(CODE_BG),
                ));
            }
            Event::SoftBreak => {
                let style = self.cur_style();
                self.spans.push(Span::styled(" ", style));
            }
            Event::HardBreak => {
                self.flush_line();
                self.start_block_line();
            }
            Event::Rule => {
                self.ensure_blank();
                self.lines
                    .push(Line::styled("─".repeat(40), Style::default().fg(DIM)));
                self.lines.push(Line::from(""));
            }
            Event::TaskListMarker(done) => {
                self.spans
                    .push(Span::raw(if done { "[x] " } else { "[ ] " }));
            }
            // Html / InlineHtml / FootnoteReference / etc. — ignored.
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                if self.list_stack.is_empty() {
                    self.ensure_blank();
                    self.start_block_line();
                }
                // Inside a list item the line is already open from Tag::Item.
            }
            Tag::Heading { level, .. } => {
                self.ensure_blank();
                self.start_block_line();
                let hashes = match level {
                    HeadingLevel::H1 => "# ",
                    HeadingLevel::H2 => "## ",
                    HeadingLevel::H3 => "### ",
                    _ => "#### ",
                };
                self.spans
                    .push(Span::styled(hashes.to_string(), Style::default().fg(ACCENT)));
                self.push_style(|s| s.fg(ACCENT).add_modifier(Modifier::BOLD));
            }
            Tag::BlockQuote(_) => self.blockquote += 1,
            Tag::CodeBlock(_) => {
                self.ensure_blank();
                self.in_code_block = true;
                self.start_block_line();
                self.spans.push(Span::raw("    "));
            }
            Tag::List(start) => {
                if self.line_has_content() {
                    self.flush_line();
                }
                self.list_stack.push(start);
            }
            Tag::Item => {
                if self.line_has_content() {
                    self.flush_line();
                }
                self.start_block_line();
                let marker = match self.list_stack.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_string(),
                };
                self.spans
                    .push(Span::styled(marker, Style::default().fg(ACCENT)));
            }
            Tag::Emphasis => self.push_style(|s| s.add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(|s| s.add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(|s| s.add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => {
                self.link_url = Some(dest_url.to_string());
                self.push_style(|s| s.fg(ACCENT).add_modifier(Modifier::UNDERLINED));
            }
            // Images, tables, footnotes, etc. — content flows through as text.
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if self.list_stack.is_empty() {
                    self.flush_line();
                }
            }
            TagEnd::Heading(_) => {
                self.style_stack.pop();
                self.flush_line();
            }
            TagEnd::BlockQuote(_) => self.blockquote = self.blockquote.saturating_sub(1),
            TagEnd::CodeBlock => {
                self.flush_line();
                self.in_code_block = false;
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::Item => {
                if self.line_has_content() {
                    self.flush_line();
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.style_stack.pop();
            }
            TagEnd::Link => {
                self.style_stack.pop();
                if let Some(url) = self.link_url.take() {
                    self.spans
                        .push(Span::styled(format!(" ({url})"), Style::default().fg(DIM)));
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Text<'static> {
        if self.line_has_content() {
            self.flush_line();
        }
        // Trim leading/trailing blank lines.
        while self
            .lines
            .first()
            .is_some_and(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
        {
            self.lines.remove(0);
        }
        while self
            .lines
            .last()
            .is_some_and(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
        {
            self.lines.pop();
        }
        Text::from(self.lines)
    }
}

/// Minimal HTML→text fallback if `htmd` conversion fails: drop tags and decode a
/// few common entities.
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_common_markdown_without_panic() {
        let md = "# Title\n\nSome **bold** and *italic* and `code`.\n\n- one\n- two\n\n> quote\n\n[link](https://example.com)";
        let text = markdown_to_text(md);
        assert!(!text.lines.is_empty());
        // The heading text should be present somewhere.
        let joined: String = text
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(joined.contains("Title"));
        assert!(joined.contains("bold"));
        assert!(joined.contains("example.com"));
    }

    #[test]
    fn html_body_becomes_markdown() {
        let md = body_to_markdown(Some("html"), "<p>Hello <strong>world</strong></p>");
        assert!(md.contains("world"));
        // plain text passes through untouched
        assert_eq!(body_to_markdown(Some("text"), "a * b"), "a * b");
    }
}
