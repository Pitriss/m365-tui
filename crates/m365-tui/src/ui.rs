//! All rendering. Pure function of `&App` — no state mutation here.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui::Frame;

use crate::app::{
    filter_commands, App, Compose, ComposeKind, Overlay, OutlookFocus, Screen, TeamsFocus,
    TeamsMode,
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
    let line = Line::from(vec![
        tab("Outlook (F2)", app.screen == Screen::Outlook),
        Span::raw("  "),
        tab("Teams (F2)", app.screen == Screen::Teams),
        Span::raw("   "),
        Span::styled("Ctrl+P palette · ? help · q quit", Style::default().fg(DIM)),
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
    let reading = if let Some(m) = &app.outlook.reading {
        let mut lines = vec![
            kv("Subject", &m.subject.clone().unwrap_or_default()),
            kv("From", &m.sender_name()),
            kv("Date", &m.received_date_time.clone().unwrap_or_default()),
            Line::raw(""),
        ];
        let body = m
            .body
            .as_ref()
            .and_then(|b| b.content.clone())
            .or_else(|| m.body_preview.clone())
            .unwrap_or_default();
        for l in strip_html(&body).lines() {
            lines.push(Line::raw(l.to_string()));
        }
        Paragraph::new(lines).wrap(Wrap { trim: false })
    } else {
        Paragraph::new("Select a message and press Enter to read.\n\nKeys: c compose · r reply · / search · g calendar · Tab focus")
            .style(Style::default().fg(DIM))
    };
    f.render_widget(
        reading.block(panel_block("Reading", app.outlook_focus == OutlookFocus::Reading)),
        cols[2],
    );
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

    // Right: messages + composer
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(cols[1]);

    let mut lines: Vec<Line> = Vec::new();
    for m in &app.teams.messages {
        if m.deleted_date_time.is_some() {
            continue;
        }
        let ts = m
            .created_date_time
            .as_deref()
            .map(|t| t.split('T').nth(1).unwrap_or(t).trim_end_matches('Z'))
            .unwrap_or("");
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", m.author()),
                Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD),
            ),
            Span::styled(ts.to_string(), Style::default().fg(DIM)),
        ]));
        for l in strip_html(&m.text()).lines() {
            lines.push(Line::raw(format!("  {l}")));
        }
    }
    if lines.is_empty() {
        lines.push(Line::styled(
            "Select a conversation and press Enter.",
            Style::default().fg(DIM),
        ));
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel_block("Conversation", app.teams.focus == TeamsFocus::Messages)),
        right[0],
    );

    let composer_text = if app.teams.composer.is_empty() {
        Span::styled("press i to type · Enter to send", Style::default().fg(DIM))
    } else {
        Span::raw(app.teams.composer.clone())
    };
    f.render_widget(
        Paragraph::new(Line::from(composer_text))
            .block(panel_block("Message", app.teams.focus == TeamsFocus::Composer)),
        right[1],
    );
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
 Global:  F2 switch app · Ctrl+P palette · ? help · q / Ctrl+C quit\n\
 \n\
 Outlook: Tab cycle panes · j/k move · Enter open · c compose\n\
          r reply · / search · g calendar\n\
 \n\
 Teams:   Tab cycle panes · j/k move · Enter open · t chats/channels\n\
          Esc / ← / h back to list · i type message · Enter send\n\
 \n\
 Compose: Tab next field · Ctrl+S send · Esc cancel\n\
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
        Overlay::Compose(c) => render_compose(f, c),
    }
}

fn render_compose(f: &mut Frame, c: &Compose) {
    let area = centered(70, 70, f.area());
    f.render_widget(Clear, area);
    let is_reply = matches!(c.kind, ComposeKind::ReplyMail { .. });
    let title = if is_reply { "Reply" } else { "Compose mail" };
    let block = popup_block(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let constraints = if is_reply {
        vec![Constraint::Min(0), Constraint::Length(1)]
    } else {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ]
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let cur = |field: usize| {
        if c.field == field {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        }
    };

    if is_reply {
        f.render_widget(
            Paragraph::new(c.body.clone())
                .style(cur(2))
                .wrap(Wrap { trim: false }),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "Ctrl+S send · Esc cancel",
                Style::default().fg(DIM),
            )),
            rows[1],
        );
    } else {
        f.render_widget(Paragraph::new(format!("To:      {}", c.to)).style(cur(0)), rows[0]);
        f.render_widget(Paragraph::new(format!("Subject: {}", c.subject)).style(cur(1)), rows[1]);
        f.render_widget(Paragraph::new("Body:").style(cur(2)), rows[2]);
        f.render_widget(
            Paragraph::new(c.body.clone()).style(cur(2)).wrap(Wrap { trim: false }),
            rows[3],
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "Tab next field · Ctrl+S send · Esc cancel",
                Style::default().fg(DIM),
            )),
            rows[4],
        );
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

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

/// Very small HTML-to-text: drop tags and decode a few common entities. Good
/// enough for reading mail/messages in a terminal.
fn strip_html(s: &str) -> String {
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
