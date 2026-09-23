//! All rendering. Pure function of `&App` — no state mutation here.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use ratatui_image::{Resize, StatefulImage};

use m365_core::models::SystemEventClass;

use crate::app::{
    filter_commands, App, CalendarView, Compose, OutlookFocus, Overlay, Screen, TeamsFocus,
    TeamsMode, NTFY_SNOOZE_HOURS, POLL_SECONDS, POLL_STALE_SECONDS,
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
        Screen::Calendar => render_calendar(f, chunks[1], app),
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
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ))),
        rows[0],
    );

    let lines = match app.screen {
        Screen::Outlook => email_lines(app).unwrap_or_default(),
        // Copy mode has no sticky date row, so keep the first inline day
        // separator there.
        Screen::Teams => conversation_lines(app, false, true).0,
        Screen::Calendar => Vec::new(),
    };
    let (wrapped, _) = crate::wrap::wrap_all(&lines, rows[1].width as usize);
    let max = (wrapped.len() as u16).saturating_sub(rows[1].height);
    f.render_widget(
        Paragraph::new(wrapped).scroll((app.copy_scroll.min(max), 0)),
        rows[1],
    );
}

/// Top row: which app is active on the left, live state on the right.
/// No key hints live here — those belong in the bottom bar.
fn render_tabs(f: &mut Frame, area: Rect, app: &App) {
    let tab = |name: &str, active: bool, unread: bool| {
        let style = if active {
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else if unread {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD)
        };
        Span::styled(format!(" {name} "), style)
    };
    let outlook_unread = app
        .outlook
        .folders
        .iter()
        .any(|folder| folder.unread_item_count.unwrap_or(0) > 0);
    let outlook_tab = if outlook_unread {
        "Outlook (F1) *"
    } else {
        "Outlook (F1)  "
    };
    let teams_has_unread = app
        .teams
        .chat_unread_counts
        .values()
        .any(|count| *count > 0);
    let teams_tab = if app.teams_unread {
        "Teams (F2) *"
    } else if teams_has_unread {
        "Teams (F2) •"
    } else {
        "Teams (F2)  "
    };
    let tabs = Line::from(vec![
        tab(outlook_tab, app.screen == Screen::Outlook, outlook_unread),
        Span::raw("  "),
        tab(
            teams_tab,
            app.screen == Screen::Teams,
            app.teams_unread || teams_has_unread,
        ),
        Span::raw("  "),
        tab("Calendar (F3)  ", app.screen == Screen::Calendar, false),
    ]);

    // Right-hand state: presence · poll health · ntfy · memory · local clock.
    let (dot, avail) = presence_indicator(app);
    let (poll_label, poll_colour) = poll_indicator(app.poll_elapsed());
    let (ntfy_label, ntfy_colour) = ntfy_indicator(app);
    let ram = match app.rss_kb {
        Some(kb) if kb >= 1024 => format!("{:.0} MB", kb as f64 / 1024.0),
        Some(kb) => format!("{kb} KB"),
        None => "—".to_string(),
    };
    let clock = chrono::Local::now().format("%H:%M").to_string();
    let sep = || Span::styled(" · ", Style::default().fg(DIM));
    let state = Line::from(vec![
        Span::styled(format!("{dot} {avail}"), presence_style(app)),
        sep(),
        Span::styled(poll_label, Style::default().fg(poll_colour)),
        sep(),
        Span::styled(ntfy_label, Style::default().fg(ntfy_colour)),
        sep(),
        Span::styled(format!("rss {ram}"), Style::default().fg(Color::Gray)),
        sep(),
        Span::styled(clock, Style::default().fg(Color::Green)),
        Span::raw(" "),
    ]);

    let state_w = line_width(&state).min(area.width);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(state_w)])
        .split(area);
    f.render_widget(Paragraph::new(tabs), cols[0]);
    f.render_widget(Paragraph::new(state), cols[1]);
}

/// Bottom row: the latest transient message on the left, the keys available
/// right now on the right.
fn render_status(f: &mut Frame, area: Rect, app: &App) {
    let bg = Color::Rgb(30, 30, 40);
    let hints = format!(" {} · ? help ", context_hints(app));
    let hints_w = (hints.chars().count() as u16).min(area.width);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(hints_w)])
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {}", app.status),
            Style::default().fg(Color::White).bg(bg),
        )))
        .style(Style::default().bg(bg)),
        cols[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hints,
            Style::default().fg(DIM).bg(bg),
        ))),
        cols[1],
    );
}

fn line_width(line: &Line) -> u16 {
    line.spans
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>() as u16
}

fn bar_with_label(label: &str) -> String {
    const INNER: usize = 10;
    let label_len = label.chars().count().min(INNER);
    let remaining = INNER.saturating_sub(label_len);
    let left = remaining / 2;
    let right = remaining - left;
    format!("[{}{}{}]", "█".repeat(left), label, "█".repeat(right))
}

fn poll_indicator(elapsed: std::time::Duration) -> (String, Color) {
    let seconds = elapsed.as_secs();

    if seconds >= POLL_STALE_SECONDS {
        return (bar_with_label("STALE"), Color::Red);
    }

    if seconds > POLL_SECONDS {
        let late = seconds - POLL_SECONDS;
        let label = if late < 60 {
            format!("+{late}s")
        } else {
            format!("+{}m", late / 60)
        };
        return (bar_with_label(&label), Color::Yellow);
    }

    let filled = ((seconds.saturating_mul(10) / POLL_SECONDS) as usize).min(10);
    (
        format!("[{}{}]", "█".repeat(filled), "░".repeat(10 - filled)),
        DIM,
    )
}

fn format_snooze_remaining(remaining: std::time::Duration) -> String {
    let minutes = remaining.as_secs().saturating_add(59) / 60;
    if minutes >= 60 {
        format!("N:{}h{:02}", minutes / 60, minutes % 60)
    } else {
        format!("N:{}m", minutes.max(1))
    }
}

fn ntfy_indicator(app: &App) -> (String, Color) {
    let (label, colour) = if !app.session.config.ntfy.enabled() {
        (String::new(), DIM)
    } else if let Some(remaining) = app.ntfy_snooze_remaining() {
        (format_snooze_remaining(remaining), Color::Red)
    } else {
        ("NTFY ON".to_string(), Color::White)
    };

    // Fixed 7-column slot: "NTFY ON" and the longest snooze form "N:24h00"
    // both fit exactly, so neighbouring status fields never jump.
    (format!("{label:<7}"), colour)
}

/// Key hints for whatever currently has focus.
fn context_hints(app: &App) -> &'static str {
    if let Some(overlay) = &app.overlay {
        return match overlay {
            Overlay::Notice(_) => "any key dismisses · auto-closes in 3s",
            Overlay::Compose(_) => "Ctrl+S send · Esc cancel",
            Overlay::Links => "1-9 open · y copy · Esc close",
            Overlay::Attachments => "1-9 save · Esc close",
            Overlay::React => "1-7 react · Esc close",
            Overlay::Presence => "1-6 set · c clear · Esc close",
            Overlay::NtfySnooze { .. } => "j/k choose · Enter apply · c/0 resume · Esc close",
            Overlay::Search { .. } => "Enter search · Esc cancel",
            Overlay::Palette { .. } => "↑↓ choose · Enter run · Esc close",
            Overlay::CalendarEvent => "o open meeting · Esc close",
            Overlay::ContactProfile => "j/k scroll · Esc close",
            Overlay::Calendar => "Esc close",
            Overlay::Help => "j/k scroll · PgUp/PgDn · Esc close",
        };
    }
    match app.screen {
        Screen::Outlook => match app.outlook_focus {
            OutlookFocus::Folders => "j/k move · l open folder",
            OutlookFocus::Messages => {
                "j/k move · l read · u read/unread · h back · c compose · r reply · / search"
            }
            OutlookFocus::Reading => {
                "j/k scroll · u read/unread · h back · o links · A attach · y copy"
            }
        },
        Screen::Teams => match app.teams.focus {
            TeamsFocus::List => "j/k preview cache · l/Enter open · g profile · t chats/channels",
            TeamsFocus::Messages => "j/k select · h back · r reply · e react · i write",
            TeamsFocus::Composer => "Enter send · Shift+Enter newline · Esc leave",
        },
        Screen::Calendar => match app.calendar.view {
            CalendarView::Agenda => "j/k move · Enter/g detail · o join · n today · a/d/t RSVP · r refresh · w range · v month",
            CalendarView::Month => "j/k event · Enter/g detail · o join · n today · ←/→ month · a/d/t RSVP · v agenda",
        },
    }
}

// ---------------------------------------------------------------------------
// Outlook
// ---------------------------------------------------------------------------

/// Split the tree decoration encoded by the folder loader from the actual
/// Outlook folder name. Tree glyphs are rendered separately so they can use
/// the same unobtrusive DIM colour as the polling progress indicator.
fn split_folder_tree_label(label: &str) -> (&str, &str) {
    let split = label
        .char_indices()
        .find(|(_, ch)| !matches!(ch, '│' | '├' | '└' | ' '))
        .map(|(index, _)| index)
        .unwrap_or(label.len());
    label.split_at(split)
}

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
    let folder_inner_width = cols[0].width.saturating_sub(3) as usize;
    let items: Vec<ListItem> = app
        .outlook
        .folders
        .iter()
        .map(|folder| {
            let unread = folder.unread_item_count.unwrap_or(0);
            let full_name = folder.display_name.as_deref().unwrap_or("");
            let (tree, name) = split_folder_tree_label(full_name);
            let suffix = match unread {
                0 => String::new(),
                1..=99 => format!("[{unread}]"),
                _ => "[+]".to_string(),
            };
            let tree_width = tree.chars().count();
            let suffix_width = suffix.chars().count();
            let name_width = folder_inner_width.saturating_sub(tree_width + suffix_width);
            let name = truncate(name, name_width);
            let padding =
                folder_inner_width.saturating_sub(tree_width + name.chars().count() + suffix_width);

            ListItem::new(Line::from(vec![
                Span::styled(tree.to_string(), Style::default().fg(DIM)),
                Span::raw(name),
                Span::raw(" ".repeat(padding)),
                Span::raw(suffix),
            ]))
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
            let clip = if m.has_attachments.unwrap_or(false) {
                "📎"
            } else {
                ""
            };
            let subject = m.subject.clone().unwrap_or_else(|| "(no subject)".into());
            let line = Line::from(vec![
                Span::styled(format!("{marker} "), Style::default().fg(ACCENT)),
                Span::styled(
                    truncate(&m.sender_name(), 18),
                    Style::default().fg(Color::LightGreen),
                ),
                Span::raw("  "),
                Span::styled(clip.to_string(), Style::default().fg(DIM)),
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
        selectable_list(
            msgs,
            &msg_title,
            app.outlook_focus == OutlookFocus::Messages,
        ),
        cols[1],
        &mut mstate,
    );

    // Reading pane — scrollable when focused, like the Teams conversation.
    let focused = app.outlook_focus == OutlookFocus::Reading;
    let title = if focused {
        "Reading (j/k scroll · Esc back)"
    } else {
        "Reading"
    };
    let block = panel_block(title, focused);
    let inner = block.inner(cols[2]);
    f.render_widget(block, cols[2]);

    // Kitty preview is additive: without decoded images, text uses the exact
    // same reading area as before.
    let image_height = if app.outlook.reading_images.is_empty() || inner.height < 8 {
        0
    } else {
        (inner.height / 3).clamp(4, 12)
    };

    let (image_area, text_area) = if image_height > 0 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(image_height), Constraint::Min(1)])
            .split(inner);
        (Some(rows[0]), rows[1])
    } else {
        (None, inner)
    };

    if let Some(image_area) = image_area {
        let count = app.outlook.reading_images.len().min(4);
        if count > 0 {
            let constraints = vec![Constraint::Ratio(1, count as u32); count];
            let image_cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(constraints)
                .split(image_area);

            for (mail_image, area) in app
                .outlook
                .reading_images
                .iter()
                .take(count)
                .zip(image_cols.iter().copied())
            {
                if let Ok(mut state) = mail_image.state.try_borrow_mut() {
                    f.render_stateful_widget(
                        StatefulImage::new().resize(Resize::Fit(None)),
                        area,
                        &mut *state,
                    );
                }
            }
        }
    }

    match email_lines(app) {
        Some(lines) => {
            let (rows, _) = crate::wrap::wrap_all(&lines, text_area.width as usize);
            // Tell the key handler how far it can usefully scroll.
            app.reading_max_scroll
                .set((rows.len() as u16).saturating_sub(text_area.height));
            let scroll = app.outlook.reading_scroll.min(app.reading_max_scroll.get());
            f.render_widget(Paragraph::new(rows).scroll((scroll, 0)), text_area);
        }
        None => {
            app.reading_max_scroll.set(0);
            f.render_widget(
                // Keys live in the bottom bar; keep the pane itself uncluttered.
                Paragraph::new("Select a message and press Enter to read.")
                    .wrap(Wrap { trim: false })
                    .style(Style::default().fg(DIM)),
                inner,
            );
        }
    }
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
    if !app.outlook.reading_attachments.is_empty() {
        let names: Vec<String> = app
            .outlook
            .reading_attachments
            .iter()
            .map(|a| format!("{} ({})", a.display_name(), a.human_size()))
            .collect();
        lines.insert(
            3,
            kv(
                "Attach",
                &format!("📎 {}  — press A to save", names.join(", ")),
            ),
        );
    }
    if let Some(body) = &app.outlook.reading_body {
        lines.extend(body.lines.iter().cloned());
    }
    Some(lines)
}

fn is_teams_inline_image_attachment_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
}

/// Lines of the open Teams conversation, plus the starting line index of each
/// message. `selectable` adds the `▶` cursor and selection highlight (off in
/// copy mode so the text copies cleanly).
pub fn conversation_lines(
    app: &App,
    selectable: bool,
    show_first_day_separator: bool,
) -> (Vec<Line<'static>>, Vec<usize>) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut starts: Vec<usize> = Vec::with_capacity(app.teams.messages.len());
    // Emit a "Today"/"Yesterday"/date separator whenever the day changes.
    let mut last_day: Option<chrono::NaiveDate> = None;
    // Track the previous message so consecutive ones from the same person can
    // share a single author header.
    let mut prev: Option<(String, Option<chrono::DateTime<chrono::Local>>)> = None;

    for (i, m) in app.teams.messages.iter().enumerate() {
        if !app.teams_message_visible(m) {
            starts.push(lines.len());
            continue;
        }

        let when = local_time(m.created_date_time.as_deref());
        let mut day_changed = false;
        if let Some(when) = when {
            let day = when.date_naive();
            if last_day != Some(day) {
                let first_day = last_day.is_none();
                if !first_day {
                    lines.push(Line::from(""));
                }
                // The normal Teams pane already has a pinned/sticky date row.
                // Suppress only the first inline separator there so a freshly
                // opened conversation does not show the same date twice.
                // Copy mode has no sticky row and asks to keep it.
                if show_first_day_separator || !first_day {
                    lines.push(day_separator(&day_label(day)));
                }
                last_day = Some(day);
                day_changed = true;
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

        // Every message opens with its own local time, so a run sharing one
        // author header still shows when each line was sent. Wrapped body lines
        // line up past that gutter.
        let ts = when
            .map(|w| w.format("%H:%M").to_string())
            .unwrap_or_else(|| " ".repeat(TIME_WIDTH));
        let gutter = " ".repeat(marker.chars().count() + TIME_WIDTH + 1);
        let lead = |time_style: Style, extra: Vec<Span<'static>>| {
            let mut spans = vec![
                Span::styled(marker.to_string(), Style::default().fg(ACCENT)),
                Span::styled(format!("{ts} "), time_style),
            ];
            spans.extend(extra);
            Line::from(spans)
        };
        let normal_time_style = Style::default().fg(Color::Gray);

        if m.deleted_date_time.is_some() {
            lines.push(lead(
                normal_time_style,
                vec![Span::styled("(message deleted)", Style::default().fg(DIM))],
            ));
            prev = None; // a deletion breaks the run
            continue;
        }

        if let Some(event) = m.system_event_display() {
            let style = match event.class {
                SystemEventClass::Useful => Style::default().fg(Color::LightRed),
                SystemEventClass::Noise | SystemEventClass::Unknown => Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            };
            lines.push(lead(
                style,
                vec![Span::styled(format!("── {}", event.text), style)],
            ));
            prev = None; // system events deliberately break an author run
            continue;
        }

        let author = m.author();
        let grouped = !day_changed
            && prev
                .as_ref()
                .is_some_and(|(a, t)| continues_run(a, *t, &author, when));

        let mut body: Vec<Line<'static>> = app
            .teams
            .messages_rendered
            .get(i)
            .map(|b| b.lines.clone())
            .unwrap_or_default();

        // A reply carries the message it answers as a `messageReference`
        // attachment, not as HTML, so it has to be drawn explicitly — and it
        // has to come *before* the reply text to read correctly.
        let quote = m.quoted().map(|q| {
            vec![
                Span::styled("┃ ", Style::default().fg(ACCENT)),
                Span::styled(
                    format!("{}: ", q.author),
                    Style::default().fg(Color::LightGreen),
                ),
                Span::styled(truncate(&q.preview, 70), Style::default().fg(DIM)),
            ]
        });

        match (grouped, quote) {
            // Grouped reply: the quote takes the lead line, the text follows.
            (true, Some(quote)) => lines.push(lead(normal_time_style, quote)),
            // Grouped message: the text starts right after the time.
            (true, None) => {
                let first = if body.is_empty() {
                    Vec::new()
                } else {
                    body.remove(0).spans
                };
                lines.push(lead(normal_time_style, first));
            }
            // New author: name on the lead line, then the quote if there is one.
            (false, quote) => {
                lines.push(lead(
                    normal_time_style,
                    vec![Span::styled(
                        author.clone(),
                        Style::default()
                            .fg(if selected {
                                Color::Cyan
                            } else {
                                Color::LightGreen
                            })
                            .add_modifier(Modifier::BOLD),
                    )],
                ));
                if let Some(quote) = quote {
                    let mut spans = vec![Span::raw(gutter.clone())];
                    spans.extend(quote);
                    lines.push(Line::from(spans));
                }
            }
        }

        for line in body {
            let mut spans = vec![Span::raw(gutter.clone())];
            spans.extend(line.spans);
            lines.push(Line::from(spans));
        }
        for att in &m.attachments {
            if let Some(name) = &att.name {
                // Attachment rows are added here, after the rendered HTML body.
                // Hide an image attachment only after the selected message has
                // at least one successfully decoded Kitty image; until then the
                // normal attachment row remains the fallback.
                let replaced_by_inline_image = i == app.teams.msg_sel
                    && !app.teams.selected_images.is_empty()
                    && is_teams_inline_image_attachment_name(name);
                if replaced_by_inline_image {
                    continue;
                }

                lines.push(Line::from(vec![
                    Span::raw(gutter.clone()),
                    Span::styled(format!("📎 {name}"), Style::default().fg(Color::LightBlue)),
                ]));
            }
        }
        if let Some(reactions) = m.reactions_summary() {
            lines.push(Line::from(vec![
                Span::raw(gutter.clone()),
                Span::styled(reactions, Style::default().fg(DIM)),
            ]));
        }
        prev = Some((author, when));
    }
    (lines, starts)
}

/// Width of the `HH:MM` timestamp column.
const TIME_WIDTH: usize = 5;

/// Whether a message continues the previous one's run: same author, and close
/// enough in time that repeating the name would just be noise.
fn continues_run(
    prev_author: &str,
    prev_at: Option<chrono::DateTime<chrono::Local>>,
    author: &str,
    at: Option<chrono::DateTime<chrono::Local>>,
) -> bool {
    if prev_author != author {
        return false;
    }
    match (prev_at, at) {
        // Messages are newest-first, so the gap can run either way.
        (Some(a), Some(b)) => (a - b).num_minutes().abs() <= RUN_GAP_MINUTES,
        _ => true,
    }
}

/// A pause this long starts a fresh header even for the same person.
const RUN_GAP_MINUTES: i64 = 15;

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

fn calendar_local_datetime(
    value: &m365_core::models::DateTimeTimeZone,
) -> Option<chrono::DateTime<chrono::Local>> {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&value.date_time) {
        return Some(parsed.with_timezone(&chrono::Local));
    }

    let naive = chrono::NaiveDateTime::parse_from_str(&value.date_time, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(&value.date_time, "%Y-%m-%dT%H:%M:%S"))
        .ok()?;

    Some(
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc)
            .with_timezone(&chrono::Local),
    )
}

fn calendar_response_marker(event: &m365_core::models::Event) -> (&'static str, Color) {
    if event.is_cancelled.unwrap_or(false) {
        return ("!", Color::Red);
    }
    if event.is_organizer.unwrap_or(false) {
        return ("O", ACCENT);
    }

    let response = event
        .response_status
        .as_ref()
        .and_then(|status| status.response.as_deref())
        .unwrap_or("")
        .to_ascii_lowercase();

    match response.as_str() {
        "accepted" => ("A", Color::Green),
        "tentativelyaccepted" => ("T", Color::Yellow),
        "declined" => ("D", Color::Red),
        "notresponded" | "none" => ("?", Color::Yellow),
        "organizer" => ("O", ACCENT),
        _ => (" ", DIM),
    }
}

fn calendar_response_label(event: &m365_core::models::Event) -> &'static str {
    if event.is_cancelled.unwrap_or(false) {
        return "cancelled";
    }
    if event.is_organizer.unwrap_or(false) {
        return "organizer";
    }

    let response = event
        .response_status
        .as_ref()
        .and_then(|status| status.response.as_deref())
        .unwrap_or("");

    if response.eq_ignore_ascii_case("accepted") {
        "accepted"
    } else if response.eq_ignore_ascii_case("tentativelyAccepted") {
        "tentative"
    } else if response.eq_ignore_ascii_case("declined") {
        "declined"
    } else if response.eq_ignore_ascii_case("notResponded") || response.eq_ignore_ascii_case("none")
    {
        "waiting"
    } else if response.eq_ignore_ascii_case("organizer") {
        "organizer"
    } else {
        "unknown"
    }
}

// Join availability is independent of RSVP and the isOnlineMeeting flag.
fn calendar_has_join_url(event: &m365_core::models::Event) -> bool {
    event
        .online_meeting
        .as_ref()
        .and_then(|meeting| meeting.join_url.as_deref())
        .is_some_and(|url| !url.trim().is_empty())
}

fn calendar_needs_response(event: &m365_core::models::Event) -> bool {
    !event.is_cancelled.unwrap_or(false)
        && !event.is_organizer.unwrap_or(false)
        && event
            .response_status
            .as_ref()
            .and_then(|status| status.response.as_deref())
            .is_some_and(|response| {
                response.eq_ignore_ascii_case("notResponded")
                    || response.eq_ignore_ascii_case("none")
            })
}

fn calendar_time_label(event: &m365_core::models::Event) -> String {
    if event.is_all_day.unwrap_or(false) {
        return "all day    ".to_string();
    }

    let start = event
        .start
        .as_ref()
        .and_then(calendar_local_datetime)
        .map(|value| value.format("%H:%M").to_string())
        .unwrap_or_else(|| "--:--".into());
    let end = event
        .end
        .as_ref()
        .and_then(calendar_local_datetime)
        .map(|value| value.format("%H:%M").to_string())
        .unwrap_or_else(|| "--:--".into());
    format!("{start}-{end}")
}

fn calendar_day_label(event: &m365_core::models::Event) -> String {
    event
        .start
        .as_ref()
        .and_then(calendar_local_datetime)
        .map(|value| value.format("%d.%m.").to_string())
        .unwrap_or_else(|| "--.--.".into())
}

fn calendar_plain_line(event: &m365_core::models::Event, show_day: bool) -> String {
    let day = if show_day {
        calendar_day_label(event)
    } else {
        "      ".into()
    };
    let time = calendar_time_label(event);
    let marker = calendar_response_marker(event).0;
    let meeting = if calendar_has_join_url(event) {
        "M"
    } else {
        " "
    };
    let subject = event.subject.as_deref().unwrap_or("(no subject)");
    format!("{day}  {time:<11}  [{marker}] [{meeting}] {subject}")
}

fn calendar_month_first_ui(offset: i32) -> chrono::NaiveDate {
    let today = chrono::Local::now().date_naive();
    let month_index =
        chrono::Datelike::year(&today) * 12 + chrono::Datelike::month0(&today) as i32 + offset;
    let year = month_index.div_euclid(12);
    let month = month_index.rem_euclid(12) as u32 + 1;
    chrono::NaiveDate::from_ymd_opt(year, month, 1).expect("valid calendar month")
}

fn calendar_month_columns(width: u16) -> usize {
    match width {
        272.. => 4,
        204..=271 => 3,
        136..=203 => 2,
        _ => 1,
    }
}

fn calendar_month_day_widths(inner_width: usize) -> [usize; 7] {
    let usable = inner_width.saturating_sub(6);
    let base = usable / 7;
    let remainder = usable % 7;
    let mut widths = [base; 7];
    for width in widths.iter_mut().take(remainder) {
        *width += 1;
    }
    widths
}

fn calendar_event_date_range(
    event: &m365_core::models::Event,
) -> Option<(
    chrono::NaiveDate,
    chrono::NaiveDate,
    chrono::DateTime<chrono::Local>,
)> {
    let start = event.start.as_ref().and_then(calendar_local_datetime)?;
    let end = event.end.as_ref().and_then(calendar_local_datetime)?;
    let start_date = start.date_naive();
    let mut end_date = end.date_naive();

    if end_date > start_date
        && end.time() == chrono::NaiveTime::from_hms_opt(0, 0, 0).expect("valid midnight")
    {
        end_date -= chrono::Duration::days(1);
    }
    if end_date < start_date {
        end_date = start_date;
    }

    Some((start_date, end_date, start))
}

fn calendar_month_event_style(event: &m365_core::models::Event, selected: bool) -> Style {
    let color = if event.is_cancelled.unwrap_or(false) {
        Color::DarkGray
    } else if event.is_organizer.unwrap_or(false) {
        ACCENT
    } else {
        let response = event
            .response_status
            .as_ref()
            .and_then(|status| status.response.as_deref())
            .unwrap_or("");

        if response.eq_ignore_ascii_case("accepted") {
            Color::Green
        } else if response.eq_ignore_ascii_case("tentativelyAccepted") {
            Color::Yellow
        } else if response.eq_ignore_ascii_case("declined") {
            Color::Red
        } else if response.eq_ignore_ascii_case("notResponded")
            || response.eq_ignore_ascii_case("none")
        {
            Color::Yellow
        } else {
            Color::Blue
        }
    };

    let foreground = if matches!(color, Color::Green | Color::Yellow | Color::Cyan) {
        Color::Black
    } else {
        Color::White
    };

    let mut style = Style::default().fg(foreground).bg(color);
    if selected {
        style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED | Modifier::REVERSED);
    } else if calendar_needs_response(event) {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

fn calendar_month_separator(widths: &[usize; 7]) -> Line<'static> {
    let mut spans = Vec::new();
    for (day, width) in widths.iter().enumerate() {
        if day > 0 {
            spans.push(Span::styled("┼", Style::default().fg(DIM)));
        }
        spans.push(Span::styled("─".repeat(*width), Style::default().fg(DIM)));
    }
    Line::from(spans)
}

fn calendar_month_center(value: &str, width: usize) -> String {
    let value = truncate(value, width);
    let used = value.chars().count();
    let padding = width.saturating_sub(used);
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", " ".repeat(left), value, " ".repeat(right))
}

fn render_calendar_month_panel(
    f: &mut Frame,
    area: Rect,
    app: &App,
    month_offset: i32,
    active: bool,
) {
    let first = calendar_month_first_ui(month_offset);
    let leading = chrono::Datelike::weekday(&first).num_days_from_monday() as i64;
    let grid_start = first - chrono::Duration::days(leading);
    let today = chrono::Local::now().date_naive();

    let inner_width = area.width.saturating_sub(2) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;
    let widths = calendar_month_day_widths(inner_width);

    if widths.iter().any(|width| *width < 5) || inner_height < 19 {
        f.render_widget(
            Paragraph::new("Window too small for month view.").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(first.format("%B %Y").to_string()),
            ),
            area,
        );
        return;
    }

    let event_rows = ((inner_height.saturating_sub(13)) / 6).clamp(1, 4);
    let weekdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let mut lines: Vec<Line<'static>> = Vec::new();

    let mut header = Vec::new();
    for (day, width) in widths.iter().enumerate() {
        if day > 0 {
            header.push(Span::styled("│", Style::default().fg(DIM)));
        }
        header.push(Span::styled(
            calendar_month_center(weekdays[day], *width),
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(header));
    lines.push(calendar_month_separator(&widths));

    for week in 0..6usize {
        let week_start = grid_start + chrono::Duration::days((week * 7) as i64);
        let week_end = week_start + chrono::Duration::days(6);

        let mut candidates: Vec<(usize, chrono::NaiveDate, chrono::NaiveDate)> = app
            .calendar
            .events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| {
                let (start, end, _) = calendar_event_date_range(event)?;
                (start <= week_end && end >= week_start).then_some((index, start, end))
            })
            .collect();
        candidates.sort_by_key(|(_, start, end)| {
            (
                *start,
                std::cmp::Reverse(end.signed_duration_since(*start).num_days()),
            )
        });

        let mut lane_masks = vec![0u8; event_rows];
        let mut segments: Vec<(usize, usize, usize, usize)> = Vec::new();
        let mut overflow = [0usize; 7];

        for (event_index, event_start, event_end) in candidates {
            let visible_start = if event_start > week_start {
                event_start
            } else {
                week_start
            };
            let visible_end = if event_end < week_end {
                event_end
            } else {
                week_end
            };
            let start_day = visible_start.signed_duration_since(week_start).num_days() as usize;
            let end_day = visible_end.signed_duration_since(week_start).num_days() as usize;

            let mut mask = 0u8;
            for day in start_day..=end_day {
                mask |= 1u8 << day;
            }

            if let Some((lane, occupied)) = lane_masks
                .iter_mut()
                .enumerate()
                .find(|(_, occupied)| (**occupied & mask) == 0)
            {
                *occupied |= mask;
                segments.push((event_index, lane, start_day, end_day));
            } else {
                for count in overflow.iter_mut().take(end_day + 1).skip(start_day) {
                    *count += 1;
                }
            }
        }

        let mut dates = Vec::new();
        for (day, width) in widths.iter().enumerate() {
            if day > 0 {
                dates.push(Span::styled("│", Style::default().fg(DIM)));
            }
            let date = week_start + chrono::Duration::days(day as i64);
            let in_month = chrono::Datelike::month(&date) == chrono::Datelike::month(&first);
            let label = if overflow[day] > 0 {
                format!("{} +{}", chrono::Datelike::day(&date), overflow[day])
            } else {
                chrono::Datelike::day(&date).to_string()
            };
            let label = truncate(&label, *width);
            let text = format!("{:<width$}", label, width = *width);

            let style = if date == today {
                Style::default()
                    .fg(Color::Black)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else if in_month {
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM)
            };
            dates.push(Span::styled(text, style));
        }
        lines.push(Line::from(dates));

        for lane in 0..event_rows {
            let mut row = Vec::new();

            for (day, width) in widths.iter().enumerate() {
                if day > 0 {
                    let bridge = segments.iter().find(|(_, segment_lane, start, end)| {
                        *segment_lane == lane && day > *start && day <= *end
                    });
                    if let Some((event_index, _, _, _)) = bridge {
                        let event = &app.calendar.events[*event_index];
                        row.push(Span::styled(
                            " ",
                            calendar_month_event_style(
                                event,
                                active && *event_index == app.calendar.selected,
                            ),
                        ));
                    } else {
                        row.push(Span::styled("│", Style::default().fg(DIM)));
                    }
                }

                let segment = segments.iter().find(|(_, segment_lane, start, end)| {
                    *segment_lane == lane && day >= *start && day <= *end
                });

                if let Some((event_index, _, start_day, end_day)) = segment {
                    let event = &app.calendar.events[*event_index];
                    let (event_start, event_end, start_time) =
                        calendar_event_date_range(event).expect("validated event range");
                    let date = week_start + chrono::Duration::days(day as i64);
                    let first_visible_day = day == *start_day;
                    let last_visible_day = day == *end_day;

                    let mut label = if first_visible_day {
                        let mut prefix = String::new();
                        if event_start < week_start {
                            prefix.push_str("◀ ");
                        } else if !event.is_all_day.unwrap_or(false) && event_start == date {
                            prefix.push_str(&start_time.format("%H:%M ").to_string());
                        }
                        if calendar_has_join_url(event) {
                            prefix.push_str("M ");
                        }
                        if active && *event_index == app.calendar.selected {
                            prefix.push_str("▶ ");
                        }
                        prefix + event.subject.as_deref().unwrap_or("(no subject)")
                    } else if active && *event_index == app.calendar.selected {
                        "═".repeat(*width)
                    } else {
                        "━".repeat(*width)
                    };

                    if last_visible_day && event_end > week_end && *width >= 2 {
                        label.push_str(" ▶");
                    }

                    let label = truncate(&label, *width);
                    row.push(Span::styled(
                        format!("{:<width$}", label, width = *width),
                        calendar_month_event_style(
                            event,
                            active && *event_index == app.calendar.selected,
                        ),
                    ));
                } else {
                    row.push(Span::raw(" ".repeat(*width)));
                }
            }

            lines.push(Line::from(row));
        }

        if week < 5 {
            lines.push(calendar_month_separator(&widths));
        }
    }

    let title = if active {
        format!("{} · active", first.format("%B %Y"))
    } else {
        first.format("%B %Y").to_string()
    };

    f.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_calendar_month(f: &mut Frame, area: Rect, app: &App) {
    let columns = calendar_month_columns(area.width);
    let gap = 1u16;
    let total_gap = gap * columns.saturating_sub(1) as u16;
    let usable = area.width.saturating_sub(total_gap);
    let base_width = usable / columns as u16;
    let remainder = usable % columns as u16;

    let mut x = area.x;
    for column in 0..columns {
        let width = base_width + if column < remainder as usize { 1 } else { 0 };
        let panel = Rect {
            x,
            y: area.y,
            width,
            height: area.height,
        };
        render_calendar_month_panel(
            f,
            panel,
            app,
            app.calendar.month_offset + column as i32,
            column == 0,
        );
        x = x.saturating_add(width).saturating_add(gap);
    }
}

fn render_calendar(f: &mut Frame, area: Rect, app: &App) {
    if app.calendar.view == CalendarView::Month {
        render_calendar_month(f, area, app);
        return;
    }

    let mut previous_day = String::new();
    let items: Vec<ListItem> = app
        .calendar
        .events
        .iter()
        .map(|event| {
            let day = calendar_day_label(event);
            let show_day = day != previous_day;
            previous_day = day;
            let (marker, marker_color) = calendar_response_marker(event);
            let day = if show_day {
                calendar_day_label(event)
            } else {
                "      ".into()
            };
            let meeting = if calendar_has_join_url(event) {
                "M"
            } else {
                " "
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{day}  "),
                    Style::default()
                        .fg(Color::Gray)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{:<11}  ", calendar_time_label(event))),
                Span::styled(
                    format!("[{marker}] "),
                    Style::default()
                        .fg(marker_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("[{meeting}] "),
                    if meeting == "M" {
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(DIM)
                    },
                ),
                Span::styled(
                    event
                        .subject
                        .as_deref()
                        .unwrap_or("(no subject)")
                        .to_string(),
                    if calendar_needs_response(event) {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]))
        })
        .collect();

    let items = if items.is_empty() {
        vec![ListItem::new(
            "No events in the next 7 days (or still loading).",
        )]
    } else {
        items
    };

    let mut state = ListState::default();
    if !app.calendar.events.is_empty() {
        state.select(Some(
            app.calendar
                .selected
                .min(app.calendar.events.len().saturating_sub(1)),
        ));
    }

    f.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Calendar — next {} days", app.calendar.days)),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
        area,
        &mut state,
    );
}

// ---------------------------------------------------------------------------
// Teams
// ---------------------------------------------------------------------------

fn contact_presence_marker(
    presence: Option<&m365_core::models::Presence>,
) -> (&'static str, Color) {
    let availability = presence
        .and_then(|p| p.availability.as_deref())
        .unwrap_or("")
        .to_ascii_lowercase();
    let activity = presence
        .and_then(|p| p.activity.as_deref())
        .unwrap_or("")
        .to_ascii_lowercase();

    if activity == "presenting" || availability == "donotdisturb" {
        ("×", Color::Red)
    } else if activity == "inacall" || activity == "inameeting" || availability.starts_with("busy")
    {
        ("●", Color::Red)
    } else if availability == "away" || availability == "berightback" {
        ("◐", Color::Yellow)
    } else if availability.starts_with("available") {
        ("●", Color::LightGreen)
    } else {
        ("○", Color::DarkGray)
    }
}

fn render_teams(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(32), Constraint::Min(20)])
        .split(area);

    let me_id = app.me.as_ref().map(|m| m.id.as_str());

    // Left list: chats or channels
    //
    // Chat rows render their selection explicitly instead of relying on the
    // generic List highlight style. The generic style sets one foreground for
    // the whole selected row, which would overwrite the contact-presence
    // colour. Explicit row styling keeps the same selection background while
    // allowing the presence glyph to retain its status colour.
    match app.teams.mode {
        TeamsMode::Chats => {
            let focused = app.teams.focus == TeamsFocus::List;
            let selected_bg = if focused { ACCENT } else { Color::Gray };
            let inner_width = cols[0].width.saturating_sub(3) as usize;

            let items: Vec<ListItem> = app
                .teams
                .chats
                .iter()
                .enumerate()
                .map(|(index, c)| {
                    let selected = index == app.teams.chat_sel;
                    let selection_style = if selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(selected_bg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let marker = if selected { "▏" } else { " " };
                    let label = app
                        .teams
                        .contact_names
                        .get(&c.id)
                        .cloned()
                        .unwrap_or_else(|| c.label(me_id));
                    let one_on_one = c
                        .chat_type
                        .as_deref()
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("oneOnOne"));
                    let user_id = app
                        .teams
                        .contact_user_ids
                        .get(&c.id)
                        .map(String::as_str)
                        .or_else(|| c.peer_user_id(me_id));

                    let unread = if one_on_one {
                        app.teams
                            .chat_unread_counts
                            .get(&c.id)
                            .copied()
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    let unread_suffix = match unread {
                        0 => String::new(),
                        1..=99 => format!("[{unread}]"),
                        _ => "[+]".to_string(),
                    };
                    let external = app.teams.external_chats.contains(&c.id);
                    let suffix = match (external, unread_suffix.is_empty()) {
                        (true, true) => "↗".to_string(),
                        (true, false) => format!("↗ {unread_suffix}"),
                        (false, _) => unread_suffix,
                    };
                    let suffix_width = suffix.chars().count();

                    // Group/meeting chats have no presence. One-to-one chats keep
                    // a fixed two-cell presence prefix even when the external
                    // user's presence cannot be retrieved, so names never shift.
                    if !app.session.config.presence_read || !one_on_one {
                        let label_width = inner_width.saturating_sub(suffix_width);
                        let label = truncate(&label, label_width);
                        let padding =
                            inner_width.saturating_sub(label.chars().count() + suffix_width);
                        return ListItem::new(Line::from(vec![
                            Span::styled(marker, selection_style),
                            Span::styled(label, selection_style),
                            Span::styled(" ".repeat(padding), selection_style),
                            Span::styled(suffix, selection_style),
                        ]));
                    }

                    let presence = user_id.and_then(|id| app.teams.contact_presences.get(id));
                    let (symbol, color) = contact_presence_marker(presence);
                    let prefix_width = 2usize;
                    let label_width = inner_width.saturating_sub(prefix_width + suffix_width);
                    let label = truncate(&label, label_width);
                    let padding = inner_width
                        .saturating_sub(prefix_width + label.chars().count() + suffix_width);
                    let presence_style = if selected {
                        // A one-cell dark badge gives the presence colour strong
                        // contrast against the cyan/gray selected-row background.
                        let selected_color = match color {
                            Color::Red => Color::LightRed,
                            Color::Yellow => Color::LightYellow,
                            Color::DarkGray => Color::White,
                            other => other,
                        };
                        Style::default()
                            .fg(selected_color)
                            .bg(Color::Black)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(color)
                    };

                    ListItem::new(Line::from(vec![
                        Span::styled(marker, selection_style),
                        Span::styled(symbol, presence_style),
                        Span::styled(" ", selection_style),
                        Span::styled(label, selection_style),
                        Span::styled(" ".repeat(padding), selection_style),
                        Span::styled(suffix, selection_style),
                    ]))
                })
                .collect();

            let mut state = ListState::default();
            state.select(Some(app.teams.chat_sel));
            f.render_stateful_widget(
                List::new(items).block(panel_block("Chats (t→channels)", focused)),
                cols[0],
                &mut state,
            );
        }
        TeamsMode::Channels => {
            let (title, items, sel): (&str, Vec<ListItem>, usize) = if app.teams.channels.is_empty()
            {
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
            };

            let mut state = ListState::default();
            state.select(Some(sel));
            f.render_stateful_widget(
                selectable_list(items, title, app.teams.focus == TeamsFocus::List),
                cols[0],
                &mut state,
            );
        }
    }

    // Right: messages + composer. The composer grows with its content (handy for
    // multi-line pastes) up to a cap; Min(5) leaves room for the border, the
    // pinned date header, and a couple of message rows on small terminals.
    let composer_width = cols[1].width.saturating_sub(2).max(1) as usize;
    let composer_rows = app.teams.composer.wrap(composer_width).len().clamp(1, 6) as u16;
    // One extra row while a reply is being composed, for the quoted banner.
    let reply_row = u16::from(app.teams.replying_to.is_some());
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(composer_rows + reply_row + 2),
        ])
        .split(cols[1]);

    let focused = app.teams.focus == TeamsFocus::Messages;
    let previewing = app.teams.focus == TeamsFocus::List && app.teams.preview_chat_id.is_some();
    let title = if previewing {
        "Conversation preview · cached".to_string()
    } else if app.teams.unseen > 0 {
        format!("Conversation — ▼ {} new (g to jump)", app.teams.unseen)
    } else if focused {
        "Conversation (j/k select · e react · z copy-mode)".to_string()
    } else {
        "Conversation".to_string()
    };

    // Build the final conversation rectangle first. Inline image sizing must use
    // the exact width available inside the border, not the whole terminal.
    let block = panel_block(&title, focused);
    let inner = block.inner(right[0]);
    f.render_widget(block, right[0]);
    let pane = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let inner_w = pane[1].width.max(1) as usize;
    let teams_image_count = app.teams.selected_images.len().min(4);
    let teams_image_height = if teams_image_count == 0 || pane[1].height == 0 {
        0
    } else {
        // Use the real source aspect ratio and terminal cell metrics even in a
        // short pane. Reserve at least one text row and let the image shrink to
        // whatever height is actually available.
        let available_height = pane[1].height;
        let max_image_height = (available_height.saturating_mul(3) / 5)
            .max(1)
            .min(available_height.saturating_sub(1).max(1));
        let image_col_width = (pane[1].width / teams_image_count as u16).max(1);
        let available = Rect::new(0, 0, image_col_width, max_image_height);

        app.teams
            .selected_images
            .iter()
            .take(teams_image_count)
            .filter_map(|image| {
                image
                    .state
                    .try_borrow()
                    .ok()
                    .map(|state| state.size_for(Resize::Fit(None), available).height)
            })
            .max()
            .unwrap_or(0)
            .min(max_image_height)
    };

    // The dedicated pinned row above the conversation already carries the
    // first visible day. Keep later inline separators for day boundaries.
    let (mut lines, msg_starts) = conversation_lines(app, focused, false);
    if lines.is_empty() {
        let empty = if previewing && app.teams.messages.is_empty() {
            "No cached conversation for the selected chat."
        } else if !app.teams.messages.is_empty() {
            "No visible messages with the current Teams system-event filter."
        } else {
            "Select a conversation and press Enter."
        };
        lines.push(Line::styled(empty, Style::default().fg(DIM)));
    }

    // Wrap first, then reserve terminal rows immediately after the selected
    // message. This makes the Kitty image part of the scrolling conversation
    // instead of a detached gallery stuck to the bottom of the panel.
    let (mut rows, row_of_line) = crate::wrap::wrap_all(&lines, inner_w);
    let mut msg_rows: Vec<usize> = msg_starts
        .iter()
        .map(|&line| row_of_line.get(line).copied().unwrap_or(rows.len()))
        .collect();

    let inline_image_row = if teams_image_height > 0 && !msg_rows.is_empty() {
        let insert_at = msg_rows
            .get(app.teams.msg_sel + 1)
            .copied()
            .unwrap_or(rows.len());

        for _ in 0..teams_image_height {
            rows.insert(insert_at, Line::raw(""));
        }

        for row in msg_rows.iter_mut().skip(app.teams.msg_sel + 1) {
            *row += teams_image_height as usize;
        }

        Some(insert_at)
    } else {
        None
    };

    let pane_h = pane[1].height.max(1) as usize;
    let sel_end = inline_image_row
        .map(|row| row + teams_image_height as usize)
        .or_else(|| msg_rows.get(app.teams.msg_sel + 1).copied())
        .unwrap_or(rows.len());
    let scroll = sel_end
        .saturating_sub(pane_h)
        .min(rows.len().saturating_sub(pane_h)) as u16;

    if let Some(label) = sticky_day_label(app, &msg_rows, scroll) {
        f.render_widget(Paragraph::new(day_separator(&label)), pane[0]);
    }

    // Text first; the graphics protocol image is drawn over its reserved blank
    // rows afterwards.
    f.render_widget(Paragraph::new(rows).scroll((scroll, 0)), pane[1]);

    if let Some(image_row) = inline_image_row {
        let image_height = teams_image_height as usize;
        let viewport_top = scroll as usize;
        let viewport_bottom = viewport_top + pane[1].height as usize;

        // Selection scrolling should keep the whole image visible. If the
        // terminal is resized mid-frame and it no longer fits, wait for the next
        // frame rather than rescaling a clipped fragment.
        if image_row >= viewport_top
            && image_row + image_height <= viewport_bottom
            && teams_image_height > 0
        {
            let image_area = Rect {
                x: pane[1].x,
                y: pane[1].y + (image_row - viewport_top) as u16,
                width: pane[1].width,
                height: teams_image_height,
            };
            f.render_widget(Clear, image_area);

            let constraints =
                vec![Constraint::Ratio(1, teams_image_count as u32); teams_image_count];
            let image_cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(constraints)
                .split(image_area);

            for (teams_image, area) in app
                .teams
                .selected_images
                .iter()
                .take(teams_image_count)
                .zip(image_cols.iter().copied())
            {
                if let Ok(mut state) = teams_image.state.try_borrow_mut() {
                    let bounds = Rect::new(0, 0, area.width, area.height);
                    let fitted = state.size_for(Resize::Fit(None), bounds);
                    if fitted.width == 0 || fitted.height == 0 {
                        continue;
                    }

                    // Small images are easier to follow when they start where
                    // the message text starts instead of floating in the middle
                    // of a wide conversation pane. Larger images stay centered.
                    let small_image =
                        fitted.width.saturating_mul(5) <= area.width.saturating_mul(3);
                    let fitted_x = if small_image {
                        // Selected message gutter: "▶ " + HH:MM + trailing space.
                        // Apply it for a single inline image so the image aligns
                        // with the message body rather than the pane border.
                        let text_indent = if teams_image_count == 1 {
                            (TIME_WIDTH as u16 + 3).min(area.width.saturating_sub(fitted.width))
                        } else {
                            0
                        };
                        area.x + text_indent
                    } else {
                        area.x + area.width.saturating_sub(fitted.width) / 2
                    };
                    let fitted_area = Rect::new(
                        fitted_x,
                        area.y + area.height.saturating_sub(fitted.height) / 2,
                        fitted.width.min(area.width),
                        fitted.height.min(area.height),
                    );
                    f.render_stateful_widget(
                        StatefulImage::new().resize(Resize::Fit(None)),
                        fitted_area,
                        &mut *state,
                    );

                    if let Some(Err(error)) = state.last_encoding_result() {
                        tracing::warn!("Teams inline image encoding failed: {error}");
                    }
                }
            }
        }
    }

    let composing = app.teams.focus == TeamsFocus::Composer;
    let title = if composing {
        "Message (Enter send · Shift/Alt+Enter newline)"
    } else {
        "Message"
    };
    let composer_block = panel_block(title, composing);
    let mut composer_inner = composer_block.inner(right[1]);
    f.render_widget(composer_block, right[1]);

    // Show what's being replied to, so the quote isn't a surprise on send.
    if let Some(idx) = app.teams.replying_to {
        let banner = Rect {
            height: 1,
            ..composer_inner
        };
        composer_inner = Rect {
            y: composer_inner.y + 1,
            height: composer_inner.height.saturating_sub(1),
            ..composer_inner
        };
        let who = app
            .teams
            .messages
            .get(idx)
            .map(|m| m.author())
            .unwrap_or_default();
        let excerpt = app
            .teams
            .messages_rendered
            .get(idx)
            .map(|t| truncate(&crate::content::plain(t).replace('\n', " "), 60))
            .unwrap_or_default();
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("┃ replying to ", Style::default().fg(ACCENT)),
                Span::styled(who, Style::default().fg(Color::LightGreen)),
                Span::styled(format!(": {excerpt}"), Style::default().fg(DIM)),
            ])),
            banner,
        );
    }

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
    } else if let Some((x, y)) = render_text_area(f, composer_inner, &app.teams.composer, composing)
    {
        f.set_cursor_position((x, y));
    }
}

// ---------------------------------------------------------------------------
// Overlays
// ---------------------------------------------------------------------------

fn calendar_event_detail(event: &m365_core::models::Event) -> Vec<Line<'static>> {
    let subject = event
        .subject
        .as_deref()
        .unwrap_or("(no subject)")
        .to_string();
    let date = event
        .start
        .as_ref()
        .and_then(calendar_local_datetime)
        .map(|value| value.format("%A %d.%m.%Y").to_string())
        .unwrap_or_else(|| "unknown".into());
    let time = if event.is_all_day.unwrap_or(false) {
        "all day".to_string()
    } else {
        calendar_time_label(event)
    };

    let organizer = event
        .organizer
        .as_ref()
        .and_then(|recipient| recipient.email_address.as_ref())
        .map(|address| {
            address
                .name
                .clone()
                .or_else(|| address.address.clone())
                .unwrap_or_else(|| "unknown".into())
        })
        .unwrap_or_else(|| "unknown".into());

    let location = event
        .location
        .as_ref()
        .and_then(|location| location.display_name.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "-".into());

    let (_, response_color) = calendar_response_marker(event);
    let response = calendar_response_label(event);
    let online = event
        .online_meeting
        .as_ref()
        .and_then(|meeting| meeting.join_url.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if event.is_online_meeting.unwrap_or(false) {
                "online meeting".into()
            } else {
                "-".into()
            }
        });

    let preview = event
        .body_preview
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();

    let mut lines = vec![
        Line::from(Span::styled(
            subject,
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Date:      ", Style::default().fg(Color::Gray)),
            Span::raw(date),
        ]),
        Line::from(vec![
            Span::styled("Time:      ", Style::default().fg(Color::Gray)),
            Span::raw(time),
        ]),
        Line::from(vec![
            Span::styled("Status:    ", Style::default().fg(Color::Gray)),
            Span::styled(
                response.to_string(),
                Style::default()
                    .fg(response_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Organizer: ", Style::default().fg(Color::Gray)),
            Span::raw(organizer),
        ]),
        Line::from(vec![
            Span::styled("Location:  ", Style::default().fg(Color::Gray)),
            Span::raw(location),
        ]),
        Line::from(vec![
            Span::styled("Meeting:   ", Style::default().fg(Color::Gray)),
            Span::raw(online),
        ]),
    ];

    if !preview.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Preview",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(preview));
    }

    lines
}

fn contact_profile_field(
    lines: &mut Vec<Line<'static>>,
    label: &'static str,
    value: Option<String>,
) {
    let Some(value) = value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return;
    };

    lines.push(Line::from(vec![
        Span::styled(format!("{label:<12}"), Style::default().fg(Color::Gray)),
        Span::raw(value),
    ]));
}

fn contact_profile_lines(app: &App) -> Vec<Line<'static>> {
    let Some(profile) = app.contact_profile.as_ref() else {
        return vec![Line::raw("No contact selected.")];
    };

    let presence = profile
        .user_id
        .as_deref()
        .and_then(|id| app.teams.contact_presences.get(id));
    let (symbol, color) = contact_presence_marker(presence);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{symbol} "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                profile.display_name.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::raw(""),
    ];

    if let Some(presence) = presence {
        let status = match (
            presence.availability.as_deref(),
            presence.activity.as_deref(),
        ) {
            (Some(availability), Some(activity))
                if !availability.eq_ignore_ascii_case(activity) =>
            {
                Some(format!("{availability} · {activity}"))
            }
            (Some(availability), _) => Some(availability.to_string()),
            (_, Some(activity)) => Some(activity.to_string()),
            _ => None,
        };
        contact_profile_field(&mut lines, "Status:", status);
    }

    if let Some(person) = profile.person.as_ref() {
        contact_profile_field(&mut lines, "Position:", person.job_title.clone());
        contact_profile_field(&mut lines, "Department:", person.department.clone());
        contact_profile_field(&mut lines, "Company:", person.company_name.clone());
        contact_profile_field(&mut lines, "Office:", person.office_location.clone());

        let email = person
            .scored_email_addresses
            .iter()
            .find_map(|address| address.address.clone())
            .or_else(|| person.user_principal_name.clone())
            .or_else(|| profile.fallback_email.clone());
        contact_profile_field(&mut lines, "E-mail:", email);

        let phones = person
            .phones
            .iter()
            .filter_map(|phone| {
                let number = phone.number.as_deref()?.trim();
                if number.is_empty() {
                    return None;
                }
                Some(match phone.phone_type.as_deref() {
                    Some(kind) if !kind.trim().is_empty() => {
                        format!("{number} ({kind})")
                    }
                    _ => number.to_string(),
                })
            })
            .collect::<Vec<_>>();
        for (index, phone) in phones.into_iter().enumerate() {
            contact_profile_field(
                &mut lines,
                if index == 0 { "Phone:" } else { "" },
                Some(phone),
            );
        }

        contact_profile_field(&mut lines, "IM:", person.im_address.clone());
    } else {
        contact_profile_field(&mut lines, "E-mail:", profile.fallback_email.clone());
    }

    contact_profile_field(&mut lines, "Account:", Some(profile.account.clone()));

    if profile.loading {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Loading profile…",
            Style::default().fg(DIM),
        ));
    }

    lines
}

fn render_overlay(f: &mut Frame, app: &App, overlay: &Overlay) {
    match overlay {
        Overlay::Notice(message) => {
            let area = centered(66, 30, f.area());
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(format!(
                    "{message}\n\nAny key dismisses · auto-closes in 3s"
                ))
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::Yellow))
                .block(popup_block("Calendar notice")),
                area,
            );
        }
        Overlay::Help => {
            let area = centered(60, 60, f.area());
            f.render_widget(Clear, area);
            let block = popup_block("Help — j/k scroll · Esc close");
            let inner = block.inner(area);
            let text = "\
 M365 TUI — keys\n\
 \n\
 Global:  F1 Outlook · F2 Teams · F3 Calendar · F4 ntfy snooze · F5 force poll · Ctrl+P palette · p presence · ? help · q quit\n\
 \n\
 NTFY:    F4 menu · j/k choose · Enter apply · c/0 resume now · Esc cancel\n\
          Snooze: 1h · 2h · 4h · 8h · 12h · 24h · Resume now\n\
 \n\
 Links:   o list links in the message · 1-9 open in browser\n\
 Attach:  A list attachments · 1-9 save to your Downloads folder\n\
          when writing: Tab to Attach, type a path, Enter to attach\n\
 \n\
 Copying: y yank focused message · Y yank whole view\n\
          z copy mode (full-width, borderless — drag-select cleanly)\n\
 \n\
 Moving:  h/← out a pane · l/→ into it (opens what's selected)\n\
          j/k or ↑/↓ move · arrows work everywhere hjkl does\n\
 \n\
 Outlook: Enter open · u read/unread · c compose · r reply · a reply-all\n\
          f forward · / search · g calendar · in the reading pane j/k scroll\n\
 \n\
 Teams:   t chats/channels · g profile on chat · g newest in messages · e react\n\
          a/i type message · r reply to selected · Enter send\n\
 \n\
 Calendar agenda: j/k select · Enter/g detail · o open meeting · n today\n\
           a accept · d decline · t tentative · r refresh · w range · v month\n\
 Calendar month:  j/k event · Enter/g detail · o open meeting · n today\n\
           ←/→ previous/next month · a/d/t RSVP · v agenda\n\
 \n\
 Compose: Tab/Shift+Tab field · Ctrl+S send · Esc cancel\n\
          ←→↑↓ move · Ctrl+←→ by word · Home/End line · Ctrl+Home/End all\n\
          Backspace/Delete · Ctrl+W word · Ctrl+U to line start · Ctrl+K to end\n\
          Enter newline in body · paste works (bracketed paste)\n\
 \n\
 Help:    j/k or ↑/↓ scroll · PgUp/PgDn · Home/End · Esc close.";
            let lines: Vec<Line<'static>> = text
                .split('\n')
                .map(|line| Line::raw(line.to_string()))
                .collect();
            let (rows, _) = crate::wrap::wrap_all(&lines, inner.width as usize);
            let max = (rows.len() as u16).saturating_sub(inner.height);
            app.help_max_scroll.set(max);
            let scroll = app.help_scroll.min(max);

            f.render_widget(block, area);
            f.render_widget(Paragraph::new(rows).scroll((scroll, 0)), inner);
        }
        Overlay::ContactProfile => {
            let area = centered(76, 72, f.area());
            f.render_widget(Clear, area);

            let block = popup_block("Contact profile — j/k scroll · Esc close");
            let inner = block.inner(area);
            f.render_widget(block, area);

            let profile = app.contact_profile.as_ref();

            // Keep the contact text at a stable x-position regardless of whether
            // an avatar exists or has finished loading. Reserve at most 25% of
            // the profile width for the image, capped at the previous 18 cells.
            let avatar_width = (inner.width / 4).min(18);
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(avatar_width),
                    Constraint::Min(1),
                ])
                .split(inner);
            let avatar_area = columns[0];
            let text_area = columns[1];

            let lines = contact_profile_lines(app);
            let (rows, _) =
                crate::wrap::wrap_all(&lines, text_area.width.max(1) as usize);
            let max = (rows.len() as u16).saturating_sub(text_area.height);
            app.contact_profile_max_scroll.set(max);
            let scroll = app.contact_profile_scroll.min(max);

            f.render_widget(Paragraph::new(rows).scroll((scroll, 0)), text_area);

            if let Some(profile) = profile {
                if let Some(avatar) = profile.avatar.as_ref() {
                    if avatar_area.width > 0 && avatar_area.height > 0 {
                        if let Ok(mut state) = avatar.state.try_borrow_mut() {
                            let bounds = Rect::new(
                                0,
                                0,
                                avatar_area.width,
                                avatar_area.height.clamp(1, 12),
                            );
                            let fitted = state.size_for(Resize::Fit(None), bounds);
                            if fitted.width > 0 && fitted.height > 0 {
                                let image_area = Rect::new(
                                    avatar_area.x
                                        + avatar_area.width.saturating_sub(fitted.width) / 2,
                                    avatar_area.y
                                        + avatar_area.height.saturating_sub(fitted.height) / 2,
                                    fitted.width.min(avatar_area.width),
                                    fitted.height.min(avatar_area.height),
                                );
                                f.render_stateful_widget(
                                    StatefulImage::new().resize(Resize::Fit(None)),
                                    image_area,
                                    &mut *state,
                                );
                            }
                        }
                    }
                }
            }
        }
        Overlay::Calendar => {
            let area = centered(70, 70, f.area());
            f.render_widget(Clear, area);
            let mut previous_day = String::new();
            let items: Vec<ListItem> = app
                .calendar
                .events
                .iter()
                .map(|event| {
                    let day = calendar_day_label(event);
                    let show_day = day != previous_day;
                    previous_day = day;
                    ListItem::new(calendar_plain_line(event, show_day))
                })
                .collect();
            let list = if items.is_empty() {
                List::new(vec![ListItem::new(
                    "No events in the next 7 days (or still loading).",
                )])
            } else {
                List::new(items)
            };
            f.render_widget(
                list.block(popup_block(&format!(
                    "Calendar — next {} days (Esc to close)",
                    app.calendar.days
                ))),
                area,
            );
        }
        Overlay::CalendarEvent => {
            let area = centered(76, 76, f.area());
            f.render_widget(Clear, area);

            if let Some(event) = app.calendar.events.get(app.calendar.selected) {
                f.render_widget(
                    Paragraph::new(calendar_event_detail(event))
                        .block(popup_block("Calendar event — Esc to close"))
                        .wrap(Wrap { trim: false }),
                    area,
                );
            } else {
                f.render_widget(
                    Paragraph::new("No calendar event selected.")
                        .block(popup_block("Calendar event — Esc to close")),
                    area,
                );
            }
        }
        Overlay::Search { query } => {
            let area = centered(60, 20, f.area());
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(format!(
                    "Search mail:\n\n> {query}▏\n\nEnter to search · Esc to cancel"
                ))
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
                    Style::default()
                        .fg(Color::Black)
                        .bg(ACCENT)
                        .add_modifier(Modifier::BOLD),
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
                Paragraph::new(format!(
                    "React to the selected message:\n\n{picks}\n\nPress 1-7 · Esc cancel"
                ))
                .wrap(Wrap { trim: false })
                .block(popup_block("Add reaction")),
                area,
            );
        }
        Overlay::Attachments => {
            let area = centered(70, 50, f.area());
            f.render_widget(Clear, area);
            let block = popup_block("Attachments — press 1-9 to save · Esc close");
            let inner = block.inner(area);
            f.render_widget(block, area);
            let items: Vec<ListItem> = app
                .outlook
                .reading_attachments
                .iter()
                .take(9)
                .enumerate()
                .map(|(i, a)| {
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{} ", i + 1),
                            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(a.display_name()),
                        Span::styled(
                            format!(
                                "  {}  {}",
                                a.human_size(),
                                a.content_type.clone().unwrap_or_default()
                            ),
                            Style::default().fg(DIM),
                        ),
                    ]))
                })
                .collect();
            f.render_widget(List::new(items), inner);
            let hint = format!("saves to {}", crate::files::download_dir().display());
            let hint_area = Rect {
                y: inner.y + inner.height.saturating_sub(1),
                height: 1,
                ..inner
            };
            f.render_widget(
                Paragraph::new(Span::styled(hint, Style::default().fg(DIM))),
                hint_area,
            );
        }
        Overlay::Links => {
            let links = app.focused_links();
            let area = centered(80, 60, f.area());
            f.render_widget(Clear, area);
            let block = popup_block("Links — press 1-9 to open · y copy first · Esc close");
            let inner = block.inner(area);
            f.render_widget(block, area);
            let width = inner.width.saturating_sub(4).max(10) as usize;
            let items: Vec<ListItem> = links
                .iter()
                .take(9)
                .enumerate()
                .map(|(i, url)| {
                    // Wrap long URLs across lines so the whole target is visible.
                    let mut lines = vec![Line::from(vec![
                        Span::styled(
                            format!("{} ", i + 1),
                            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(host_of(url), Style::default().fg(Color::LightGreen)),
                    ])];
                    for chunk in chunks_of(url, width) {
                        lines.push(Line::styled(format!("  {chunk}"), Style::default().fg(DIM)));
                    }
                    ListItem::new(lines)
                })
                .collect();
            f.render_widget(List::new(items), inner);
        }
        Overlay::NtfySnooze { sel } => {
            let area = centered(42, 50, f.area());
            f.render_widget(Clear, area);
            let block = popup_block("ntfy snooze — F4");
            let inner = block.inner(area);
            f.render_widget(block, area);

            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min((NTFY_SNOOZE_HOURS.len() + 1) as u16),
                    Constraint::Length(1),
                ])
                .split(inner);

            let current = app
                .ntfy_snooze_remaining()
                .map(|remaining| format!("Current: {}", format_snooze_remaining(remaining)))
                .unwrap_or_else(|| "Current: no snooze".to_string());
            f.render_widget(
                Paragraph::new(Span::styled(current, Style::default().fg(Color::Gray))),
                rows[0],
            );

            let mut items: Vec<ListItem> = NTFY_SNOOZE_HOURS
                .iter()
                .map(|hours| {
                    ListItem::new(format!(
                        "{hours} hour{}",
                        if *hours == 1 { "" } else { "s" }
                    ))
                })
                .collect();
            items.push(ListItem::new("Resume now"));
            let mut state = ListState::default();
            state.select(Some((*sel).min(NTFY_SNOOZE_HOURS.len())));
            f.render_stateful_widget(
                List::new(items)
                    .highlight_style(
                        Style::default()
                            .fg(Color::Black)
                            .bg(ACCENT)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol("▏"),
                rows[1],
                &mut state,
            );
            f.render_widget(
                Paragraph::new(Span::styled(
                    "Enter set · c/0 resume now · Esc cancel",
                    Style::default().fg(DIM),
                )),
                rows[2],
            );
        }
        Overlay::Presence => {
            let area = centered(46, 55, f.area());
            f.render_widget(Clear, area);
            let mut body = String::new();
            if let Some(a) = app
                .my_presence
                .as_ref()
                .and_then(|p| p.availability.as_deref())
            {
                body.push_str(&format!("Current: {a}\n\n"));
            }
            for (i, opt) in crate::app::PRESENCE_OPTIONS.iter().enumerate() {
                body.push_str(&format!("{}  {}\n", i + 1, opt.label));
            }
            body.push_str("\nc  Clear (revert to automatic)\nEsc cancel");
            body.push_str(
                "\n\nThis app publishes its own presence session, so the status\nshows even with no Teams client running. Quitting clears it.",
            );
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

    // header rows (To/Subject) + "Body:" label + body + attach + staged + hint
    let staged = c.attachments.len() as u16;
    let mut constraints = Vec::new();
    if show_to {
        constraints.push(Constraint::Length(1));
    }
    if show_subject {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1)); // Body: label
    constraints.push(Constraint::Min(0)); // body
    constraints.push(Constraint::Length(1)); // Attach: input
    constraints.push(Constraint::Length(staged.min(4))); // staged files
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

    // Attach: type a path, Enter stages it.
    cursor = render_line_field(f, rows[i], "Attach:  ", &c.attach, c.field == 3).or(cursor);
    i += 1;

    // Staged files (most recent last), capped to the rows we reserved.
    let staged_area = rows[i];
    if staged_area.height > 0 {
        let shown = staged_area.height as usize;
        let skip = c.attachments.len().saturating_sub(shown);
        let lines: Vec<Line> = c
            .attachments
            .iter()
            .skip(skip)
            .map(|(path, size)| {
                Line::from(vec![
                    Span::styled("  📎 ", Style::default().fg(Color::LightBlue)),
                    Span::raw(
                        path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                    ),
                    Span::styled(format!("  {}", human_size(*size)), Style::default().fg(DIM)),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(lines), staged_area);
    }
    i += 1;

    let hint = if c.field == 3 {
        "Enter attach file · Ctrl+X remove last · Tab field · Ctrl+S send · Esc cancel"
    } else {
        "Tab field · ←→ move · Ctrl+←→ word · Ctrl+W/U/K delete · Ctrl+S send · Esc cancel"
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(DIM))),
        rows[i],
    );

    if let Some((x, y)) = cursor {
        f.set_cursor_position((x, y));
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// The host part of a URL, for a readable link label.
fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .to_string()
}

/// Split a long string into fixed-width chunks so it can be shown in full.
fn chunks_of(s: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    chars
        .chunks(width.max(1))
        .map(|c| c.iter().collect())
        .collect()
}

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
        Span::styled(" ".to_string() + &"─".repeat(40), Style::default().fg(DIM)),
    ])
}

fn compact_presence_label(availability: Option<&str>) -> &'static str {
    match availability.unwrap_or("") {
        "Available" | "AvailableIdle" => "Avail",
        "Busy" | "BusyIdle" => "Busy ",
        "DoNotDisturb" => "DND  ",
        "Away" => "Away ",
        "BeRightBack" => "BRB  ",
        "Offline" => "Off  ",
        _ => "…    ",
    }
}

/// Tab-bar presence uses a fixed compact 7-column slot: dot, space, 5-char label.
fn presence_indicator(app: &App) -> (&'static str, &'static str) {
    let avail = app
        .my_presence
        .as_ref()
        .and_then(|p| p.availability.as_deref());
    ("●", compact_presence_label(avail))
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

#[cfg(test)]
mod tests {
    use super::{contact_presence_marker, day_label, local_time};
    use ratatui::style::Color;

    #[test]
    fn contact_presence_marker_matches_teams_status_colors() {
        use m365_core::models::Presence;

        fn presence(availability: &str, activity: &str) -> Presence {
            Presence {
                id: None,
                availability: Some(availability.to_string()),
                activity: Some(activity.to_string()),
            }
        }

        assert_eq!(
            contact_presence_marker(Some(&presence("Available", "Available"))),
            ("●", Color::LightGreen)
        );
        assert_eq!(
            contact_presence_marker(Some(&presence("Busy", "Busy"))),
            ("●", Color::Red)
        );
        assert_eq!(
            contact_presence_marker(Some(&presence("Busy", "InAMeeting"))),
            ("●", Color::Red)
        );
        assert_eq!(
            contact_presence_marker(Some(&presence("Busy", "InACall"))),
            ("●", Color::Red)
        );
        assert_eq!(
            contact_presence_marker(Some(&presence("DoNotDisturb", "DoNotDisturb"))),
            ("×", Color::Red)
        );
        assert_eq!(
            contact_presence_marker(Some(&presence("Away", "Away"))),
            ("◐", Color::Yellow)
        );
        assert_eq!(
            contact_presence_marker(Some(&presence("BeRightBack", "BeRightBack"))),
            ("◐", Color::Yellow)
        );
        assert_eq!(
            contact_presence_marker(Some(&presence("Offline", "Offline"))),
            ("○", Color::DarkGray)
        );
    }

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
    fn groups_consecutive_messages_from_one_sender() {
        use super::continues_run;
        let at = |h, m| {
            Some(
                chrono::NaiveDate::from_ymd_opt(2026, 8, 3)
                    .unwrap()
                    .and_hms_opt(h, m, 0)
                    .unwrap()
                    .and_local_timezone(chrono::Local)
                    .unwrap(),
            )
        };

        // Same person, a minute apart: one header covers both.
        assert!(continues_run("Jaime", at(16, 30), "Jaime", at(16, 29)));
        // Different people never group.
        assert!(!continues_run("Jaime", at(16, 30), "António", at(16, 29)));
        // A long pause earns a fresh header even for the same person.
        assert!(!continues_run("Jaime", at(16, 30), "Jaime", at(15, 00)));
        // Gap is symmetric — the list runs newest-first.
        assert!(!continues_run("Jaime", at(15, 00), "Jaime", at(16, 30)));
        // Missing timestamps fall back to the author check alone.
        assert!(continues_run("Jaime", None, "Jaime", at(16, 30)));
    }

    #[test]
    fn body_lines_align_under_the_timestamp_gutter() {
        use super::TIME_WIDTH;
        // Every message opens with `marker + HH:MM + space`; wrapped body lines
        // are indented by exactly that, so text stays in one column whether or
        // not the message is grouped.
        for marker in ["▶ ", "  ", ""] {
            let lead = marker.chars().count() + TIME_WIDTH + 1;
            let gutter = " ".repeat(lead);
            assert_eq!(gutter.chars().count(), lead, "marker {marker:?}");
        }
    }

    #[test]
    fn parses_graph_timestamps_to_local() {
        assert!(local_time(Some("2026-07-27T14:30:00Z")).is_some());
        assert!(local_time(Some("2026-07-27T14:30:00.123Z")).is_some());
        assert!(local_time(Some("not a date")).is_none());
        assert!(local_time(None).is_none());
    }
}

#[cfg(test)]
mod ntfy_status_tests {
    use super::{compact_presence_label, format_snooze_remaining, poll_indicator};

    #[test]
    fn poll_indicator_keeps_normal_bar_then_reports_late_and_stale() {
        assert_eq!(
            poll_indicator(std::time::Duration::from_secs(18)).0,
            "[█████████░]"
        );
        assert_eq!(
            poll_indicator(std::time::Duration::from_secs(22)).0,
            "[███+2s████]"
        );
        assert_eq!(
            poll_indicator(std::time::Duration::from_secs(120)).0,
            "[██STALE███]"
        );
    }

    #[test]
    fn snooze_status_is_compact() {
        assert_eq!(
            format_snooze_remaining(std::time::Duration::from_secs(47 * 60)),
            "N:47m"
        );
        assert_eq!(
            format_snooze_remaining(std::time::Duration::from_secs(97 * 60)),
            "N:1h37"
        );
    }

    #[test]
    fn presence_labels_have_fixed_compact_width() {
        for (availability, expected) in [
            (Some("Available"), "Avail"),
            (Some("Busy"), "Busy "),
            (Some("DoNotDisturb"), "DND  "),
            (Some("Away"), "Away "),
            (Some("BeRightBack"), "BRB  "),
            (Some("Offline"), "Off  "),
            (None, "…    "),
        ] {
            let label = compact_presence_label(availability);
            assert_eq!(label, expected);
            assert_eq!(label.chars().count(), 5);
        }
    }
}

#[cfg(test)]
mod calendar_meeting_indicator_tests {
    use super::{calendar_has_join_url, calendar_plain_line, calendar_response_marker};
    use m365_core::models::Event;
    use serde_json::json;

    #[test]
    fn rsvp_and_join_indicators_are_independent() {
        for (response, marker) in [
            ("accepted", "A"),
            ("tentativelyAccepted", "T"),
            ("declined", "D"),
            ("notResponded", "?"),
            ("organizer", "O"),
        ] {
            for online in [false, true] {
                for (url, has_join) in [
                    (None, false),
                    (Some(""), false),
                    (Some("   "), false),
                    (Some("https://example.com/meeting"), true),
                ] {
                    let event: Event = serde_json::from_value(json!({
                        "id": "test",
                        "subject": "Example",
                        "responseStatus": {"response": response},
                        "isOnlineMeeting": online,
                        "onlineMeeting": {"joinUrl": url}
                    }))
                    .unwrap();
                    assert_eq!(calendar_response_marker(&event).0, marker);
                    assert_eq!(calendar_has_join_url(&event), has_join);
                    let join = if has_join { "M" } else { " " };
                    let line = calendar_plain_line(&event, true);
                    assert!(line.ends_with(&format!("[{marker}] [{join}] Example")));
                    assert_eq!(line.chars().count(), 36);
                }
            }
        }
    }

    #[test]
    fn organizer_and_cancelled_flags_do_not_hide_join_links() {
        for (organizer, cancelled, marker) in
            [(true, false, "O"), (false, true, "!"), (true, true, "!")]
        {
            let event: Event = serde_json::from_value(json!({
                "id": "test",
                "isOrganizer": organizer,
                "isCancelled": cancelled,
                "onlineMeeting": {"joinUrl": "https://example.com/meeting"}
            }))
            .unwrap();
            assert_eq!(calendar_response_marker(&event).0, marker);
            assert!(calendar_has_join_url(&event));
        }
    }
}
