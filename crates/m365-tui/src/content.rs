//! Turning Graph message/mail bodies into styled terminal text.
//!
//! Both Outlook mail and Teams messages arrive as HTML (or occasionally plain
//! text). We render the HTML **directly** to a styled ratatui [`Text`] by
//! walking the parsed DOM — no Markdown round-trip (that produced escaping and
//! table artifacts). The result is cached by the caller so parsing happens once,
//! not every frame.

use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const LINK: Color = Color::LightBlue;
const CODE_FG: Color = Color::Rgb(220, 185, 130);
const CODE_BG: Color = Color::Rgb(40, 40, 52);

/// Render a Graph body to styled text. HTML is parsed and walked; plain text is
/// split into lines verbatim.
pub fn render_body(content_type: Option<&str>, raw: &str) -> Text<'static> {
    let is_html = content_type
        .map(|c| c.eq_ignore_ascii_case("html"))
        .unwrap_or_else(|| looks_like_html(raw));
    if is_html {
        render_html(raw)
    } else {
        Text::from(
            raw.lines()
                .map(|l| Line::raw(l.to_string()))
                .collect::<Vec<_>>(),
        )
    }
}

fn looks_like_html(s: &str) -> bool {
    let s = s.trim_start();
    s.starts_with('<') && s.contains('>')
}

/// Parse HTML and walk the DOM into styled lines.
pub fn render_html(html: &str) -> Text<'static> {
    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap_or_default();

    let mut r = Renderer::default();
    r.walk(&dom.document, Style::default());
    r.finish()
}

#[derive(Default)]
struct Renderer {
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    blockquote: usize,
    list_stack: Vec<Option<u64>>,
    in_pre: bool,
    /// Depth of skip-containers (script/style/head) currently open.
    skip: usize,
}

impl Renderer {
    fn line_has_content(&self) -> bool {
        self.spans.iter().any(|s| !s.content.trim().is_empty())
    }

    fn flush_line(&mut self) {
        let spans = std::mem::take(&mut self.spans);
        self.lines.push(Line::from(spans));
    }

    /// Finish the current line (if it has content) and start a fresh one.
    fn newline(&mut self) {
        self.flush_line();
        self.start_block_line();
    }

    /// Blank separator unless the previous line is already blank/empty.
    fn ensure_blank(&mut self) {
        if self.line_has_content() {
            self.flush_line();
        }
        if let Some(last) = self.lines.last() {
            let blank = last.spans.iter().all(|s| s.content.trim().is_empty());
            if !blank {
                self.lines.push(Line::from(""));
            }
        }
        self.start_block_line();
    }

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

    fn push_text(&mut self, raw: &str, style: Style) {
        if self.skip > 0 {
            return;
        }
        if self.in_pre {
            let mut parts = raw.split('\n').peekable();
            while let Some(part) = parts.next() {
                self.spans
                    .push(Span::styled(part.to_string(), Style::default().fg(CODE_FG)));
                if parts.peek().is_some() {
                    self.newline();
                    self.spans.push(Span::raw("    "));
                }
            }
            return;
        }
        let mut text = collapse_ws(raw);
        if text.trim().is_empty() {
            // Keep a single separating space only if the line already has text.
            if self.line_has_content() && !self.spans.last().is_some_and(ends_with_space) {
                self.spans.push(Span::styled(" ", style));
            }
            return;
        }
        if !self.line_has_content() {
            text = text.trim_start().to_string();
        }
        self.spans.push(Span::styled(text, style));
    }

    fn walk(&mut self, node: &Handle, style: Style) {
        match &node.data {
            NodeData::Document => self.walk_children(node, style),
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                self.push_text(&text, style);
            }
            NodeData::Element { name, attrs, .. } => {
                let tag = name.local.as_ref().to_ascii_lowercase();
                self.element(&tag, node, attrs, style);
            }
            _ => {}
        }
    }

    fn walk_children(&mut self, node: &Handle, style: Style) {
        for child in node.children.borrow().iter() {
            self.walk(child, style);
        }
    }

    fn element(
        &mut self,
        tag: &str,
        node: &Handle,
        attrs: &std::cell::RefCell<Vec<html5ever::Attribute>>,
        style: Style,
    ) {
        match tag {
            "script" | "style" | "head" | "title" | "noscript" => {
                self.skip += 1;
                self.walk_children(node, style);
                self.skip -= 1;
            }
            "br" => self.newline(),
            "hr" => {
                self.ensure_blank();
                self.lines
                    .push(Line::styled("─".repeat(40), Style::default().fg(DIM)));
                self.lines.push(Line::from(""));
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "table" => {
                self.ensure_blank();
                self.walk_children(node, style);
                self.ensure_blank();
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.ensure_blank();
                let hashes = "#".repeat(tag[1..].parse::<usize>().unwrap_or(1));
                self.spans
                    .push(Span::styled(format!("{hashes} "), Style::default().fg(ACCENT)));
                self.walk_children(node, style.fg(ACCENT).add_modifier(Modifier::BOLD));
                self.ensure_blank();
            }
            "strong" | "b" => self.walk_children(node, style.add_modifier(Modifier::BOLD)),
            "em" | "i" => self.walk_children(node, style.add_modifier(Modifier::ITALIC)),
            "u" | "ins" => self.walk_children(node, style.add_modifier(Modifier::UNDERLINED)),
            "s" | "strike" | "del" => {
                self.walk_children(node, style.add_modifier(Modifier::CROSSED_OUT))
            }
            "code" if !self.in_pre => {
                // Inline code: render children as code-styled text.
                self.walk_children(node, style.fg(CODE_FG).bg(CODE_BG));
            }
            "pre" => {
                self.ensure_blank();
                self.in_pre = true;
                self.start_block_line();
                self.spans.push(Span::raw("    "));
                self.walk_children(node, style);
                self.in_pre = false;
                self.ensure_blank();
            }
            "blockquote" => {
                self.ensure_blank();
                self.blockquote += 1;
                self.walk_children(node, style.fg(DIM));
                self.blockquote = self.blockquote.saturating_sub(1);
                self.ensure_blank();
            }
            "ul" => {
                if self.line_has_content() {
                    self.flush_line();
                }
                self.list_stack.push(None);
                self.walk_children(node, style);
                self.list_stack.pop();
            }
            "ol" => {
                if self.line_has_content() {
                    self.flush_line();
                }
                self.list_stack.push(Some(1));
                self.walk_children(node, style);
                self.list_stack.pop();
            }
            "li" => {
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
                self.walk_children(node, style);
                if self.line_has_content() {
                    self.flush_line();
                }
            }
            "a" => {
                let link_style = style.fg(LINK).add_modifier(Modifier::UNDERLINED);
                self.walk_children(node, link_style);
                if let Some(href) = attr(attrs, "href") {
                    if !href.is_empty() && !href.starts_with('#') && !href.starts_with("mailto:") {
                        self.spans
                            .push(Span::styled(format!(" ({href})"), Style::default().fg(DIM)));
                    }
                }
            }
            "img" => {
                if let Some(alt) = attr(attrs, "alt") {
                    if !alt.is_empty() {
                        self.push_text(&format!("[{alt}]"), style.fg(DIM));
                    }
                }
            }
            "tr" => {
                self.walk_children(node, style);
                self.newline();
            }
            "td" | "th" => {
                self.walk_children(node, style);
                self.spans.push(Span::styled("  ", Style::default().fg(DIM)));
            }
            // Inline/neutral wrappers: keep the current style.
            _ => self.walk_children(node, style),
        }
    }

    fn finish(mut self) -> Text<'static> {
        if self.line_has_content() {
            self.flush_line();
        }
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

fn ends_with_space(s: &Span) -> bool {
    s.content.ends_with(' ')
}

fn attr(attrs: &std::cell::RefCell<Vec<html5ever::Attribute>>, name: &str) -> Option<String> {
    attrs
        .borrow()
        .iter()
        .find(|a| a.name.local.as_ref().eq_ignore_ascii_case(name))
        .map(|a| a.value.to_string())
}

/// Flatten styled text back to plain text (one string per line, trailing
/// whitespace trimmed) for the clipboard.
pub fn plain(text: &Text) -> String {
    text.lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Collapse runs of ASCII/Unicode whitespace to single spaces (HTML semantics).
fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(text: &Text) -> String {
        text.lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_html_without_tags() {
        let html = "<div><h2>Hi</h2><p>Some <b>bold</b> and <a href=\"https://x.io\">a link</a>.</p><ul><li>one</li><li>two</li></ul></div>";
        let text = render_html(html);
        let s = flat(&text);
        assert!(s.contains("Hi"));
        assert!(s.contains("bold"));
        assert!(s.contains("a link"));
        assert!(s.contains("https://x.io"));
        assert!(s.contains("• one"));
        assert!(!s.contains('<'), "tags leaked: {s}");
    }

    #[test]
    fn plain_text_passthrough() {
        let text = render_body(Some("text"), "line one\nline two * star");
        assert_eq!(flat(&text), "line one\nline two * star");
    }

    #[test]
    fn script_and_style_are_dropped() {
        let text = render_html("<style>.x{color:red}</style><p>visible</p><script>alert(1)</script>");
        let s = flat(&text);
        assert!(s.contains("visible"));
        assert!(!s.contains("alert"));
        assert!(!s.contains("color:red"));
    }
}
