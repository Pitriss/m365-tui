//! All rendering. Pure function of `&App` — no state mutation here.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui::Frame;

use crate::app::{
    filter_commands, App, Compose, Overlay, OutlookFocus, Screen, TeamsFocus, TeamsMode,
};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    // Copy mode takes over the whole frame: one hint row plus borderless,
    // full-width text so a terminal drag-select grabs only the message body.
    if app.copy_mode {
        render_copy_mode(f, app);
        return;
    }

    render_tabs(f, chunks[0], app);
    match app.screen {
        Screen::Outlook => render_outlook(f, chunks[1], app),
        Screen::Teams => render_teams(f, chunks[1], app),
    }
    render_status(f, chunks[2], app);

    if let Some(overlay) = &app.overlay {
        render_overlay(f, app, overlay);
    }
}

/// Full-screen, borderless view of the current message/conversation. No side
/// panes and no borders, so mouse selection captures exactly the text.
fn render_copy_mode(f: &mut Frame, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(f.area());

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " COPY MODE — drag to select · y yank all · j/k scroll · z/Esc exit ",
            Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD),
        ))),
        rows[0],
    );

    let lines = match app.screen {
        Screen::Outlook => email_lines(app).unwrap_or_default(),
        Screen::Teams => conversation_lines(app, false).0,
    };
    let total = lines.len() as u16;
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.copy_scroll.min(total.saturating_sub(1)), 0)),
        rows[1],
    );
}

fn render_tabs(f: &mut Frame, area: Rect, app: &App) {
    let tab = |name: &str, active: bool| {
        if active {
            Span::styled(
                format!(" {name} "),
                Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!(" {name} "), Style::default().fg(DIM))
        }
    };
    let sync = match &app.last_sync {
        Some(t) => format!("⟳ synced {t}"),
        None => "⟳ syncing…".to_string(),
    };
    let (dot, avail) = presence_indicator(app);
    let line = Line::from(vec![
        tab("Outlook (F2)", app.screen == Screen::Outlook),
        Span::raw("  "),
        tab("Teams (F2)", app.screen == Screen::Teams),
        Span::raw("   "),
        Span::styled("Ctrl+P palette · p presence · ? help · q quit   ", Style::default().fg(DIM)),
        Span::styled(format!("{dot} {avail}  "), presence_style(app)),
        Span::styled(sync, Style::default().fg(Color::Green)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_status(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            app.status.clone(),
            Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 40)),
        ))),
        area,
    );
}

// ---------------------------------------------------------------------------
// Outlook
// ---------------------------------------------------------------------------

fn render_outlook(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(26),
            Constraint::Percentage(40),
            Constraint::Min(20),
        ])
        .split(area);

    // Folders
    let items: Vec<ListItem> = app
        .outlook
        .folders
        .iter()
        .map(|folder| {
            let unread = folder.unread_item_count.unwrap_or(0);
            let name = folder.display_name.clone().unwrap_or_default();
            let label = if unread > 0 {
                format!("{name} ({unread})")
            } else {
                name
            };
            ListItem::new(label)
        })
        .collect();
    let mut fstate = ListState::default();
    fstate.select(Some(app.outlook.folder_sel));
    f.render_stateful_widget(
        selectable_list(items, "Folders", app.outlook_focus == OutlookFocus::Folders),
        cols[0],
        &mut fstate,
    );

    // Messages
    let msgs: Vec<ListItem> = app
        .outlook
        .messages
        .iter()
        .map(|m| {
            let unread = !m.is_read.unwrap_or(true);
            let marker = if unread { "●" } else { " " };
            let subject = m.subject.clone().unwrap_or_else(|| "(no subject)".into());
            let line = Line::from(vec![
                Span::styled(format!("{marker} "), Style::default().fg(ACCENT)),
                Span::styled(
                    truncate(&m.sender_name(), 18),
                    Style::default().fg(Color::LightGreen),
                ),
                Span::raw("  "),
                Span::styled(
                    subject,
                    if unread {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]);
            ListItem::new(line)
        })
        .collect();
    let mut mstate = ListState::default();
    mstate.select(Some(app.outlook.msg_sel));
    let msg_title = if app.outlook.messages_next.is_some() {
        format!("Messages ({} · ↓ for more)", app.outlook.messages.len())
    } else {
        format!("Messages ({})", app.outlook.messages.len())
    };
    f.render_stateful_widget(
        selectable_list(msgs, &msg_title, app.outlook_focus == OutlookFocus::Messages),
        cols[1],
        &mut mstate,
    );

    // Reading pane
    let reading = match email_lines(app) {
        Some(lines) => Paragraph::new(lines).wrap(Wrap { trim: false }),
        None => Paragraph::new("Select a message and press Enter to read.\n\nKeys: c compose · r reply · a reply-all · f forward · / search · g calendar · z copy-mode · y yank")
            .style(Style::default().fg(DIM)),
    };
    f.render_widget(
        reading.block(panel_block("Reading", app.outlook_focus == OutlookFocus::Reading)),
        cols[2],
    );
}

/// Headers + rendered body of the open email, or `None` if nothing is open.
/// Shared by the reading pane and copy mode.
pub fn email_lines(app: &App) -> Option<Vec<Line<'static>>> {
    let m = app.outlook.reading.as_ref()?;
    let mut lines = vec![
        kv("Subject", &m.subject.clone().unwrap_or_default()),
        kv("From", &m.sender_name()),
        kv("Date", &m.received_date_time.clone().unwrap_or_default()),
        Line::raw(""),
    ];
    if let Some(body) = &app.outlook.reading_body {
        lines.extend(body.lines.iter().cloned());
    }
    Some(lines)
}

/// Lines of the open Teams conversation, plus the starting line index of each
/// message. `selectable` adds the `▶` cursor and selection highlight (off in
/// copy mode so the text copies cleanly).
pub fn conversation_lines(app: &App, selectable: bool) -> (Vec<Line<'static>>, Vec<usize>) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut starts: Vec<usize> = Vec::with_capacity(app.teams.messages.len());
    // Emit a "Today"/"Yesterday"/date separator whenever the day changes.
    let mut last_day: Option<chrono::NaiveDate> = None;
    for (i, m) in app.teams.messages.iter().enumerate() {
        let when = local_time(m.created_date_time.as_deref());
        if let Some(when) = when {
            let day = when.date_naive();
            if last_day != Some(day) {
                if last_day.is_some() {
                    lines.push(Line::from(""));
                }
                lines.push(day_separator(&day_label(day)));
                last_day = Some(day);
            }
        }
        // Record the start *after* any separator, so scrolling to a message
        // puts the message itself at the top — the pinned header carries the
        // date, and we avoid showing the same date twice.
        starts.push(lines.len());
        let selected = selectable && i == app.teams.msg_sel;
        let marker = if !selectable {
            ""
        } else if selected {
            "▶ "
        } else {
            "  "
        };
        let indent = if selectable { "    " } else { "  " };

        if m.deleted_date_time.is_some() {
            lines.push(Line::from(vec![
                Span::styled(marker.to_string(), Style::default().fg(ACCENT)),
                Span::styled("(message deleted)", Style::default().fg(DIM)),
            ]));
            continue;
        }

        // Local wall-clock time (Graph returns UTC).
        let ts = when
            .map(|w| w.format("%H:%M").to_string())
            .unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(marker.to_string(), Style::default().fg(ACCENT)),
            Span::styled(
                format!("{} ", m.author()),
                Style::default()
                    .fg(if selected { Color::Cyan } else { Color::LightGreen })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(ts, Style::default().fg(DIM)),
        ]));
        if let Some(body) = app.teams.messages_rendered.get(i) {
            for line in &body.lines {
                let mut spans = vec![Span::raw(indent)];
                spans.extend(line.spans.iter().cloned());
                lines.push(Line::from(spans));
            }
        }
        if let Some(reactions) = m.reactions_summary() {
            lines.push(Line::from(vec![
                Span::raw(indent),
                Span::styled(reactions, Style::default().fg(DIM)),
            ]));
        }
    }
    (lines, starts)
}

// ---------------------------------------------------------------------------
// Teams
// ---------------------------------------------------------------------------

fn render_teams(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(32), Constraint::Min(20)])
        .split(area);

    let me_id = app.me.as_ref().map(|m| m.id.as_str());

    // Left list: chats or channels
    let (title, items, sel): (&str, Vec<ListItem>, usize) = match app.teams.mode {
        TeamsMode::Chats => {
            let items = app
                .teams
                .chats
                .iter()
                .map(|c| ListItem::new(truncate(&c.label(me_id), 30)))
                .collect();
            ("Chats (t→channels)", items, app.teams.chat_sel)
        }
        TeamsMode::Channels => {
            if app.teams.channels.is_empty() {
                let items = app
                    .teams
                    .teams
                    .iter()
                    .map(|t| ListItem::new(truncate(t.display_name.as_deref().unwrap_or(""), 30)))
                    .collect();
                ("Teams (Enter→channels)", items, app.teams.team_sel)
            } else {
                let items = app
                    .teams
                    .channels
                    .iter()
                    .map(|c| ListItem::new(truncate(c.display_name.as_deref().unwrap_or(""), 30)))
                    .collect();
                ("Channels (t→chats)", items, app.teams.channel_sel)
            }
        }
    };
    let mut lstate = ListState::default();
    lstate.select(Some(sel));
    f.render_stateful_widget(
        selectable_list(items, title, app.teams.focus == TeamsFocus::List),
        cols[0],
        &mut lstate,
    );

    // Right: messages + composer. The composer grows with its content (handy for
    // multi-line pastes) up to a cap; Min(5) leaves room for the border, the
    // pinned date header, and a couple of message rows on small terminals.
    let composer_width = cols[1].width.saturating_sub(2).max(1) as usize;
    let composer_rows = app.teams.composer.wrap(composer_width).len().clamp(1, 6) as u16;
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(composer_rows + 2)])
        .split(cols[1]);

    let focused = app.teams.focus == TeamsFocus::Messages;
    let (mut lines, msg_starts) = conversation_lines(app, focused);
    if lines.is_empty() {
        lines.push(Line::styled(
            "Select a conversation and press Enter.",
            Style::default().fg(DIM),
        ));
    }
    // Scroll so the selected message sits near the top of the pane.
    let total = lines.len() as u16;
    let scroll = msg_starts
        .get(app.teams.msg_sel)
        .map(|&s| s as u16)
        .unwrap_or(0)
        .min(total.saturating_sub(1));
    let title = if focused {
        "Conversation (j/k select · e react · z copy-mode)"
    } else {
        "Conversation"
    };

    // The pane is split inside its border: a pinned date header on the first
    // row, then the scrolling message flow. The header tracks the day of the
    // topmost visible message, so it updates as you scroll.
    let block = panel_block(title, focused);
    let inner = block.inner(right[0]);
    f.render_widget(block, right[0]);
    let pane = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    if let Some(label) = sticky_day_label(app, &msg_starts, scroll) {
        f.render_widget(Paragraph::new(day_separator(&label)), pane[0]);
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll, 0)),
        pane[1],
    );

    let composing = app.teams.focus == TeamsFocus::Composer;
    let title = if composing {
        "Message (Enter send · Shift/Alt+Enter newline)"
    } else {
        "Message"
    };
    let composer_block = panel_block(title, composing);
    let composer_inner = composer_block.inner(right[1]);
    f.render_widget(composer_block, right[1]);
    app.text_width_hint
        .set(composer_inner.width.max(1) as usize);
    if app.teams.composer.is_empty() && !composing {
        f.render_widget(
            Paragraph::new(Span::styled(
                "press i to type · Enter to send",
                Style::default().fg(DIM),
            )),
            composer_inner,
        );
    } else if let Some((x, y)) =
        render_text_area(f, composer_inner, &app.teams.composer, composing)
    {
        f.set_cursor_position((x, y));
    }
}

// ---------------------------------------------------------------------------
// Overlays
// ---------------------------------------------------------------------------

fn render_overlay(f: &mut Frame, app: &App, overlay: &Overlay) {
    match overlay {
        Overlay::Help => {
            let area = centered(60, 60, f.area());
            f.render_widget(Clear, area);
            let text = "\
 M365 TUI — keys\n\
 \n\
 Global:  F2 switch app · Ctrl+P palette · p set presence · ? help · q quit\n\
 \n\
 Copying: y yank focused message · Y yank whole view\n\
          z copy mode (full-width, borderless — drag-select cleanly)\n\
 \n\
 Outlook: Tab cycle panes · j/k move · Enter open · c compose\n\
          r reply · a reply-all · f forward · / search · g calendar\n\
 \n\
 Teams:   Tab cycle panes · Enter open · t chats/channels\n\
          j/k select message · e react · Esc / ← / h back to list\n\
          i type message · Enter send\n\
 \n\
 Compose: Tab/Shift+Tab field · Ctrl+S send · Esc cancel\n\
          ←→↑↓ move · Ctrl+←→ by word · Home/End line · Ctrl+Home/End all\n\
          Backspace/Delete · Ctrl+W word · Ctrl+U to line start · Ctrl+K to end\n\
          Enter newline in body · paste works (bracketed paste)\n\
 \n\
 Press Esc to close.";
            f.render_widget(
                Paragraph::new(text).block(popup_block("Help")).wrap(Wrap { trim: false }),
                area,
            );
        }
        Overlay::Calendar => {
            let area = centered(70, 70, f.area());
            f.render_widget(Clear, area);
            let items: Vec<ListItem> = app
                .outlook
                .calendar
                .iter()
                .map(|e| {
                    let start = e
                        .start
                        .as_ref()
                        .map(|s| s.date_time.replace('T', " "))
                        .unwrap_or_default();
                    let subj = e.subject.clone().unwrap_or_default();
                    let online = if e.is_online_meeting.unwrap_or(false) { " 🔗" } else { "" };
                    ListItem::new(format!("{start}  {subj}{online}"))
                })
                .collect();
            let list = if items.is_empty() {
                List::new(vec![ListItem::new("No events in the next 7 days (or still loading).")])
            } else {
                List::new(items)
            };
            f.render_widget(list.block(popup_block("Calendar — next 7 days (Esc to close)")), area);
        }
        Overlay::Search { query } => {
            let area = centered(60, 20, f.area());
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(format!("Search mail:\n\n> {query}▏\n\nEnter to search · Esc to cancel"))
                    .block(popup_block("Search")),
                area,
            );
        }
        Overlay::Palette { query, sel } => {
            let area = centered(50, 60, f.area());
            f.render_widget(Clear, area);
            let matches = filter_commands(query);
            let block = popup_block("Command palette");
            let inner = block.inner(area);
            f.render_widget(block, area);
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(2), Constraint::Min(0)])
                .split(inner);
            f.render_widget(Paragraph::new(format!("> {query}▏")), rows[0]);
            let items: Vec<ListItem> = matches
                .iter()
                .map(|(_, label)| ListItem::new(*label))
                .collect();
            let mut st = ListState::default();
            st.select(Some(*sel));
            f.render_stateful_widget(
                List::new(items).highlight_style(
                    Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                rows[1],
                &mut st,
            );
        }
        Overlay::Compose(c) => render_compose(f, c, app),
        Overlay::React => {
            let area = centered(50, 24, f.area());
            f.render_widget(Clear, area);
            let picks: String = crate::app::REACTIONS
                .iter()
                .enumerate()
                .map(|(i, e)| format!("{}  {e}   ", i + 1))
                .collect();
            f.render_widget(
                Paragraph::new(format!("React to the selected message:\n\n{picks}\n\nPress 1-7 · Esc cancel"))
                    .wrap(Wrap { trim: false })
                    .block(popup_block("Add reaction")),
                area,
            );
        }
        Overlay::Presence => {
            let area = centered(46, 55, f.area());
            f.render_widget(Clear, area);
            let mut body = String::new();
            if let Some(a) = app.my_presence.as_ref().and_then(|p| p.availability.as_deref()) {
                body.push_str(&format!("Current: {a}\n\n"));
            }
            for (i, (label, _, _)) in crate::app::PRESENCE_OPTIONS.iter().enumerate() {
                body.push_str(&format!("{}  {label}\n", i + 1));
            }
            body.push_str("\nc  Clear (revert to automatic)\nEsc cancel");
            if !app.session.config.can_write_presence() {
                body.push_str("\n\nread-only: set M365_PRESENCE_WRITE=1 and grant\nPresence.ReadWrite to enable changing status");
            }
            f.render_widget(
                Paragraph::new(body).block(popup_block("Set presence")),
                area,
            );
        }
    }
}


/// Render a wrapped, vertically-scrolling text area. Returns the on-screen
/// cursor position when focused. Shared by the compose body and the Teams
/// composer so both wrap and scroll identically.
fn render_text_area(
    f: &mut Frame,
    area: Rect,
    input: &crate::editor::TextInput,
    focused: bool,
) -> Option<(u16, u16)> {
    let width = area.width.max(1) as usize;
    let height = area.height.max(1) as usize;
    let wrapped = input.wrap(width);
    let (crow, ccol) = input.cursor_position(width);
    // Keep the cursor row on screen.
    let scroll = crow.saturating_sub(height.saturating_sub(1));
    let visible: Vec<Line> = wrapped
        .iter()
        .skip(scroll)
        .take(height)
        .map(|r| Line::raw(r.text.clone()))
        .collect();
    f.render_widget(Paragraph::new(visible), area);
    focused.then(|| {
        (
            area.x + ccol.min(width.saturating_sub(1)) as u16,
            area.y + (crow - scroll) as u16,
        )
    })
}

/// Render one single-line field, scrolling horizontally to keep the cursor in
/// view. Returns the on-screen cursor column when this field is focused.
fn render_line_field(
    f: &mut Frame,
    area: Rect,
    label: &str,
    input: &crate::editor::TextInput,
    focused: bool,
) -> Option<(u16, u16)> {
    let style = if focused {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let label_w = label.chars().count() as u16;
    let avail = area.width.saturating_sub(label_w).max(1) as usize;
    let text = input.text();
    let chars: Vec<char> = text.chars().collect();
    // Scroll so the cursor stays visible in a long recipient list.
    let offset = input.cursor().saturating_sub(avail.saturating_sub(1));
    let shown: String = chars.iter().skip(offset).take(avail).collect();

    f.render_widget(Paragraph::new(format!("{label}{shown}")).style(style), area);

    focused.then(|| {
        let col = area.x + label_w + (input.cursor() - offset) as u16;
        (col.min(area.x + area.width.saturating_sub(1)), area.y)
    })
}

fn render_compose(f: &mut Frame, c: &Compose, app: &App) {
    let area = centered(70, 70, f.area());
    f.render_widget(Clear, area);
    let block = popup_block(c.kind.title());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let fields = c.kind.fields();
    let show_to = fields.contains(&0);
    let show_subject = fields.contains(&1);

    // header rows (To/Subject) + "Body:" label + body area + hint line
    let mut constraints = Vec::new();
    if show_to {
        constraints.push(Constraint::Length(1));
    }
    if show_subject {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1)); // Body: label
    constraints.push(Constraint::Min(0)); // body
    constraints.push(Constraint::Length(1)); // hint
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let mut cursor: Option<(u16, u16)> = None;
    let mut i = 0;
    if show_to {
        cursor = render_line_field(f, rows[i], "To:      ", &c.to, c.field == 0).or(cursor);
        i += 1;
    }
    if show_subject {
        cursor = render_line_field(f, rows[i], "Subject: ", &c.subject, c.field == 1).or(cursor);
        i += 1;
    }

    let body_style = if c.field == 2 {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    f.render_widget(Paragraph::new("Body:").style(body_style), rows[i]);
    i += 1;

    let body_area = rows[i];
    // Tell the key handler what width Up/Down should move by.
    app.text_width_hint.set(body_area.width.max(1) as usize);
    cursor = render_text_area(f, body_area, &c.body, c.field == 2).or(cursor);
    i += 1;

    f.render_widget(
        Paragraph::new(Span::styled(
            "Tab field · ←→ move · Ctrl+←→ word · Ctrl+W/U/K delete · Ctrl+S send · Esc cancel",
            Style::default().fg(DIM),
        )),
        rows[i],
    );

    if let Some((x, y)) = cursor {
        f.set_cursor_position((x, y));
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Parse a Graph UTC timestamp into the local timezone.
fn local_time(ts: Option<&str>) -> Option<chrono::DateTime<chrono::Local>> {
    chrono::DateTime::parse_from_rfc3339(ts?)
        .ok()
        .map(|t| t.with_timezone(&chrono::Local))
}

/// `Today` / `Yesterday` / `Mon 21 Jul` (with the year for other years).
fn day_label(day: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    let today = chrono::Local::now().date_naive();
    if day == today {
        "Today".to_string()
    } else if Some(day) == today.pred_opt() {
        "Yesterday".to_string()
    } else if day.year() == today.year() {
        day.format("%a %-d %b").to_string()
    } else {
        day.format("%a %-d %b %Y").to_string()
    }
}

/// Day label for the message currently at the top of the visible area — the
/// content of the pinned header. `starts` is ascending, so the topmost visible
/// message is the last one starting at or above the scroll offset.
fn sticky_day_label(app: &App, starts: &[usize], scroll: u16) -> Option<String> {
    let idx = topmost_message_index(starts, scroll);
    let when = local_time(app.teams.messages.get(idx)?.created_date_time.as_deref())?;
    Some(day_label(when.date_naive()))
}

/// Index of the message occupying the top of the visible area.
fn topmost_message_index(starts: &[usize], scroll: u16) -> usize {
    starts
        .iter()
        .rposition(|&s| s <= scroll as usize)
        .unwrap_or(0)
}

fn day_separator(label: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("── ", Style::default().fg(DIM)),
        Span::styled(
            label.to_string(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " ".to_string() + &"─".repeat(40),
            Style::default().fg(DIM),
        ),
    ])
}

/// Tab-bar presence dot symbol + availability label.
fn presence_indicator(app: &App) -> (&'static str, String) {
    let avail = app
        .my_presence
        .as_ref()
        .and_then(|p| p.availability.clone())
        .unwrap_or_else(|| "…".into());
    ("●", avail)
}

fn presence_style(app: &App) -> Style {
    let color = match app
        .my_presence
        .as_ref()
        .and_then(|p| p.availability.as_deref())
        .unwrap_or("")
    {
        "Available" | "AvailableIdle" => Color::Green,
        "Busy" | "BusyIdle" | "DoNotDisturb" => Color::Red,
        "Away" | "BeRightBack" => Color::Yellow,
        _ => DIM,
    };
    Style::default().fg(color)
}

fn selectable_list<'a>(items: Vec<ListItem<'a>>, title: &'a str, focused: bool) -> List<'a> {
    List::new(items)
        .block(panel_block(title, focused))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(if focused { ACCENT } else { Color::Gray })
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▏")
}

fn panel_block(title: &str, focused: bool) -> Block<'_> {
    let color = if focused { ACCENT } else { DIM };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
}

fn popup_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
}

fn kv<'a>(k: &'a str, v: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{k}: "), Style::default().fg(DIM)),
        Span::raw(v.to_string()),
    ])
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{day_label, local_time};

    #[test]
    fn labels_relative_days() {
        let today = chrono::Local::now().date_naive();
        assert_eq!(day_label(today), "Today");
        assert_eq!(day_label(today.pred_opt().unwrap()), "Yesterday");
        // An older date renders as a weekday/day/month, not "Today".
        let old = chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap();
        let label = day_label(old);
        assert!(label.contains("Mar"), "unexpected label: {label}");
        assert!(label.contains("2024"), "past years show the year: {label}");
        assert!(!label.contains('-'), "no literal padding modifier: {label}");
    }

    #[test]
    fn sticky_header_tracks_topmost_message() {
        use super::topmost_message_index;
        // Three messages beginning at lines 0, 5 and 12.
        let starts = [0usize, 5, 12];
        assert_eq!(topmost_message_index(&starts, 0), 0);
        assert_eq!(topmost_message_index(&starts, 4), 0); // still inside msg 0
        assert_eq!(topmost_message_index(&starts, 5), 1); // exactly at msg 1
        assert_eq!(topmost_message_index(&starts, 11), 1);
        assert_eq!(topmost_message_index(&starts, 12), 2);
        assert_eq!(topmost_message_index(&starts, 99), 2); // clamped past the end
        // A separator above the first message must not select a negative index.
        assert_eq!(topmost_message_index(&[3, 9], 0), 0);
        assert_eq!(topmost_message_index(&[], 7), 0);
    }

    #[test]
    fn parses_graph_timestamps_to_local() {
        assert!(local_time(Some("2026-07-27T14:30:00Z")).is_some());
        assert!(local_time(Some("2026-07-27T14:30:00.123Z")).is_some());
        assert!(local_time(Some("not a date")).is_none());
        assert!(local_time(None).is_none());
    }
}

fn centered(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(v[1])[1]
}
