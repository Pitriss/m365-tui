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

/// A rendered message body: styled text plus the links it referenced, in the
/// order they were numbered (`[1]`, `[2]`, ...).
#[derive(Debug, Clone, Default)]
pub struct RenderedBody {
    pub text: Text<'static>,
    pub links: Vec<String>,
}

/// Render a Graph body to styled text. HTML is parsed and walked; plain text is
/// split into lines verbatim.
pub fn render_body(content_type: Option<&str>, raw: &str) -> RenderedBody {
    let is_html = content_type
        .map(|c| c.eq_ignore_ascii_case("html"))
        .unwrap_or_else(|| looks_like_html(raw));
    if is_html {
        render_html(raw)
    } else {
        render_plain(raw)
    }
}

fn render_plain(raw: &str) -> RenderedBody {
    let mut links = Vec::new();
    let lines = raw
        .lines()
        .map(|line| {
            let mut spans = Vec::new();
            push_autolink_spans(&mut spans, &mut links, line, Style::default());
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    RenderedBody {
        text: Text::from(lines),
        links,
    }
}

fn link_number_in(links: &mut Vec<String>, url: &str) -> usize {
    if let Some(index) = links.iter().position(|link| link == url) {
        return index + 1;
    }
    links.push(url.to_string());
    links.len()
}

fn find_autolink_start(text: &str) -> Option<usize> {
    // ASCII lowercasing preserves byte offsets for the schemes we search for.
    let lower = text.to_ascii_lowercase();
    ["https://", "http://", "mailto:"]
        .into_iter()
        .filter_map(|scheme| lower.find(scheme))
        .min()
}

fn trim_autolink_end(candidate: &str) -> usize {
    let mut end = candidate.len();
    while let Some(ch) = candidate[..end].chars().next_back() {
        let body = &candidate[..end];
        let strip = matches!(ch, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"')
            || (ch == ')'
                && body.chars().filter(|c| *c == ')').count()
                    > body.chars().filter(|c| *c == '(').count())
            || (ch == ']'
                && body.chars().filter(|c| *c == ']').count()
                    > body.chars().filter(|c| *c == '[').count())
            || (ch == '}'
                && body.chars().filter(|c| *c == '}').count()
                    > body.chars().filter(|c| *c == '{').count());
        if !strip {
            break;
        }
        end = end.saturating_sub(ch.len_utf8());
    }
    end
}

fn valid_autolink(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("https://") {
        return !rest.is_empty();
    }
    if let Some(rest) = lower.strip_prefix("http://") {
        return !rest.is_empty();
    }
    if let Some(rest) = lower.strip_prefix("mailto:") {
        return !rest.is_empty();
    }
    false
}

fn push_autolink_spans(
    spans: &mut Vec<Span<'static>>,
    links: &mut Vec<String>,
    text: &str,
    style: Style,
) {
    let mut cursor = 0usize;
    while cursor < text.len() {
        let Some(relative_start) = find_autolink_start(&text[cursor..]) else {
            spans.push(Span::styled(text[cursor..].to_string(), style));
            return;
        };
        let start = cursor + relative_start;
        if start > cursor {
            spans.push(Span::styled(text[cursor..start].to_string(), style));
        }

        let rest = &text[start..];
        let token_end = rest
            .char_indices()
            .find_map(|(index, ch)| ch.is_whitespace().then_some(index))
            .unwrap_or(rest.len());
        let candidate = &rest[..token_end];
        let link_end = trim_autolink_end(candidate);
        if link_end == 0 {
            // Defensive progress guarantee for malformed input.
            let ch = rest.chars().next().expect("non-empty rest");
            spans.push(Span::styled(ch.to_string(), style));
            cursor = start + ch.len_utf8();
            continue;
        }

        let displayed = &candidate[..link_end];
        if !valid_autolink(displayed) {
            spans.push(Span::styled(displayed.to_string(), style));
            cursor = start + link_end;
            continue;
        }

        let target = if displayed.to_ascii_lowercase().starts_with("http://")
            || displayed.to_ascii_lowercase().starts_with("https://")
        {
            unwrap_safelink(displayed)
        } else {
            displayed.to_string()
        };
        let number = link_number_in(links, &target);
        spans.push(Span::styled(
            displayed.to_string(),
            style.fg(LINK).add_modifier(Modifier::UNDERLINED),
        ));
        spans.push(Span::styled(
            format!("[{number}]"),
            Style::default().fg(LINK),
        ));
        cursor = start + link_end;
    }
}

fn looks_like_html(s: &str) -> bool {
    let s = s.trim_start();
    s.starts_with('<') && s.contains('>')
}

/// Parse HTML and walk the DOM into styled lines.
pub fn render_html(html: &str) -> RenderedBody {
    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap_or_default();

    let mut r = Renderer::default();
    r.walk(&dom.document, Style::default());
    let links = r.links.clone();
    RenderedBody {
        text: r.finish(),
        links,
    }
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
    /// Depth inside an explicit HTML anchor. Naked-URL autodetection is disabled
    /// there because the surrounding <a href> owns the target and numbering.
    link_depth: usize,
    /// Link targets in numbering order; the inline text shows `[n]` instead of
    /// the URL, which keeps huge Safelinks out of the reading flow.
    links: Vec<String>,
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
            prefix.push_str("┃ ");
        }
        let depth = self.list_stack.len().saturating_sub(1);
        for _ in 0..depth {
            prefix.push_str("  ");
        }
        if !prefix.is_empty() {
            self.spans
                .push(Span::styled(prefix, Style::default().fg(DIM)));
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
        if self.link_depth == 0 {
            push_autolink_spans(&mut self.spans, &mut self.links, &text, style);
        } else {
            self.spans.push(Span::styled(text, style));
        }
    }

    /// Number a link, reusing the number if the same target appears again.
    fn link_number(&mut self, url: &str) -> usize {
        if let Some(i) = self.links.iter().position(|l| l == url) {
            return i + 1;
        }
        self.links.push(url.to_string());
        self.links.len()
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
                self.spans.push(Span::styled(
                    format!("{hashes} "),
                    Style::default().fg(ACCENT),
                ));
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
                let href = attr(attrs, "href")
                    .map(|href| unwrap_safelink(href.trim()))
                    .filter(|href| !href.is_empty());
                let clickable = href.as_deref().is_some_and(|href| !href.starts_with('#'));
                let child_style = if clickable {
                    style.fg(LINK).add_modifier(Modifier::UNDERLINED)
                } else {
                    style
                };
                self.link_depth += 1;
                self.walk_children(node, child_style);
                self.link_depth = self.link_depth.saturating_sub(1);
                if let Some(href) = href.filter(|_| clickable) {
                    let n = self.link_number(&href);
                    self.spans
                        .push(Span::styled(format!("[{n}]"), Style::default().fg(LINK)));
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
                self.spans
                    .push(Span::styled("  ", Style::default().fg(DIM)));
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

/// Microsoft Defender rewrites links through
/// `*.safelinks.protection.outlook.com/?url=<encoded>&data=...`, which turns a
/// short link into ~800 characters. Recover the original target.
pub fn unwrap_safelink(url: &str) -> String {
    if !url.contains("safelinks.protection.outlook.com") {
        return url.to_string();
    }
    let Some((_, query)) = url.split_once('?') else {
        return url.to_string();
    };
    for pair in query.split('&') {
        if let Some(encoded) = pair.strip_prefix("url=") {
            let decoded = percent_decode(encoded);
            if !decoded.is_empty() {
                return decoded;
            }
        }
    }
    url.to_string()
}

fn percent_decode(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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
        let s = flat(&render_html(html).text);
        assert!(s.contains("Hi"));
        assert!(s.contains("bold"));
        assert!(s.contains("a link"));
        assert!(s.contains("• one"));
        assert!(!s.contains('<'), "tags leaked: {s}");
    }

    #[test]
    fn plain_text_passthrough() {
        let out = render_body(Some("text"), "line one\nline two * star");
        assert_eq!(flat(&out.text), "line one\nline two * star");
    }

    #[test]
    fn safelinks_are_unwrapped_to_the_real_target() {
        // Shaped exactly like a live Outlook rewrite: percent-encoded target,
        // then the tracking parameters Exchange appends after it.
        let wrapped = "https://eur03.safelinks.protection.outlook.com/?url=https%3A%2F%2Ftickets.example.org%2Fscp%2Ftickets.php%3Fid%3D47398&data=05%7C02%7Csomeone%40example.org%7C572b&reserved=0";
        assert_eq!(
            unwrap_safelink(wrapped),
            "https://tickets.example.org/scp/tickets.php?id=47398"
        );
        // Ordinary links pass through untouched.
        let plain = "https://github.com/rootHytx/m365-tui";
        assert_eq!(unwrap_safelink(plain), plain);
        // A malformed safelink must not panic or lose the original.
        let broken = "https://eur03.safelinks.protection.outlook.com/no-query-here";
        assert_eq!(unwrap_safelink(broken), broken);
    }

    #[test]
    fn links_are_numbered_and_deduped_not_inlined() {
        let html = r#"<p>Please <a href="https://example.com/login">login</a> to continue,
                      or <a href="https://example.com/login">login here</a>,
                      or visit <a href="https://other.example/x">other</a>.</p>"#;
        let out = render_html(html);
        let text = flat(&out.text);
        // The anchor text stays, the URL does not appear in the flow.
        assert!(text.contains("login"));
        assert!(
            !text.contains("https://example.com/login"),
            "url leaked: {text}"
        );
        // Markers are numbered, and the repeated target reuses its number.
        assert!(text.contains("[1]"), "{text}");
        assert!(text.contains("[2]"), "{text}");
        assert_eq!(out.links.len(), 2, "duplicate targets share a number");
        assert_eq!(out.links[0], "https://example.com/login");
        assert_eq!(out.links[1], "https://other.example/x");
    }

    #[test]
    fn plain_text_http_and_mailto_are_numbered_links() {
        let out = render_body(
            Some("text"),
            "Draft https://example.com/path. Contact mailto:user@example.com?subject=Hello%20there",
        );
        let text = flat(&out.text);
        assert!(text.contains("https://example.com/path[1]."), "{text}");
        assert!(
            text.contains("mailto:user@example.com?subject=Hello%20there[2]"),
            "{text}"
        );
        assert_eq!(
            out.links,
            vec![
                "https://example.com/path".to_string(),
                "mailto:user@example.com?subject=Hello%20there".to_string(),
            ]
        );
    }

    #[test]
    fn naked_url_inside_html_text_is_autolinked() {
        let out = render_html("<p>Draft URL: https://example.com/draft</p>");
        let text = flat(&out.text);
        assert!(text.contains("https://example.com/draft[1]"), "{text}");
        assert_eq!(out.links, vec!["https://example.com/draft"]);
    }

    #[test]
    fn explicit_anchor_is_not_double_numbered_and_mailto_is_kept() {
        let out = render_html(
            r##"<p><a href="https://example.com/x">https://example.com/x</a> <a href="mailto:user@example.com">mail</a> <a href="#local">local</a></p>"##,
        );
        let text = flat(&out.text);
        assert_eq!(text.matches("[1]").count(), 1, "{text}");
        assert_eq!(text.matches("[2]").count(), 1, "{text}");
        assert_eq!(out.links.len(), 2);
        assert_eq!(out.links[0], "https://example.com/x");
        assert_eq!(out.links[1], "mailto:user@example.com");
        let local = out
            .text
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.contains("local"))
            .unwrap();
        assert!(
            !local.style.add_modifier.contains(Modifier::UNDERLINED),
            "fragment-only anchor must not look externally clickable"
        );
    }

    #[test]
    fn renders_a_teams_reply_quote() {
        // The shape Teams uses for a reply in a chat: the original is embedded
        // as a blockquote, followed by the new text.
        let html = r#"<blockquote itemscope itemtype="http://schema.skype.com/Reply" itemid="1754321652000">
              <strong itemprop="mri">Alex Rivera</strong>
              <span itemprop="time"></span>
              <p itemprop="preview">Sounds good to me</p>
            </blockquote><p>Confirma por favor</p>"#;
        let out = render_html(html);
        let text = flat(&out.text);
        // Both the quoted original and the reply itself must survive.
        assert!(
            text.contains("Alex Rivera"),
            "quoted author missing: {text}"
        );
        assert!(
            text.contains("Sounds good to me"),
            "quoted text missing: {text}"
        );
        assert!(text.contains("Confirma por favor"), "reply missing: {text}");
        // The quote is visually marked off from the reply.
        assert!(text.contains('┃'), "quote marker missing: {text}");
    }

    #[test]
    fn attachment_tag_body_still_renders_its_text() {
        let out =
            render_html(r#"<attachment id="1785858892876"></attachment><p>Confirma por favor</p>"#);
        let s = flat(&out.text);
        assert!(s.contains("Confirma por favor"), "reply text lost: {s:?}");
    }

    #[test]
    fn script_and_style_are_dropped() {
        let out =
            render_html("<style>.x{color:red}</style><p>visible</p><script>alert(1)</script>");
        let s = flat(&out.text);
        assert!(s.contains("visible"));
        assert!(!s.contains("alert"));
        assert!(!s.contains("color:red"));
    }
}
