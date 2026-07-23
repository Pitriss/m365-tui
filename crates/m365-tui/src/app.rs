//! Application state and the async orchestration behind it.
//!
//! The UI thread never blocks on the network: key handlers spawn tokio tasks
//! that fetch from Graph and send an [`AppMessage`] back over an mpsc channel,
//! which the main loop applies to the state before the next redraw.

use std::future::Future;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use m365_core::events::{ChangeEvent, ChangeKind};
use m365_core::models::{
    Chat, ChatMessage, Event as CalEvent, MailFolder, MailMessage, Team, User,
};
use m365_core::{calendar, chats, channels, mail, Session};
use tokio::sync::mpsc;

use crate::navigation;

/// Messages sent from background tasks to the UI loop.
#[derive(Debug)]
pub enum AppMessage {
    Status(String),
    Error(String),
    Whoami(User),
    Folders(Vec<MailFolder>),
    Messages {
        items: Vec<MailMessage>,
        next: Option<String>,
        /// Append to the existing list (load-more) vs. replace it (new folder).
        append: bool,
    },
    MessageBody(MailMessage),
    Calendar(Vec<CalEvent>),
    Chats(Vec<Chat>),
    ChatMessages { chat_id: String, messages: Vec<ChatMessage> },
    Teams(Vec<Team>),
    Channels { team_id: String, channels: Vec<m365_core::models::Channel> },
    ChannelMessages {
        team_id: String,
        channel_id: String,
        messages: Vec<ChatMessage>,
    },
    /// A send/action completed; optional status text and refresh hint.
    Done(String),
    /// Result of a cross-navigation request to open a chat by email.
    OpenChat(Option<String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Outlook,
    Teams,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OutlookFocus {
    Folders,
    Messages,
    Reading,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TeamsMode {
    Chats,
    Channels,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TeamsFocus {
    List,
    Messages,
    Composer,
}

/// A transient full-screen/modal overlay.
pub enum Overlay {
    Help,
    Palette { query: String, sel: usize },
    Search { query: String },
    Compose(Compose),
    Calendar,
}

pub enum ComposeKind {
    NewMail,
    ReplyMail { id: String },
}

pub struct Compose {
    pub kind: ComposeKind,
    pub to: String,
    pub subject: String,
    pub body: String,
    /// 0 = To, 1 = Subject, 2 = Body (To/Subject hidden for replies).
    pub field: usize,
}

#[derive(Default)]
pub struct OutlookState {
    pub folders: Vec<MailFolder>,
    pub folder_sel: usize,
    pub messages: Vec<MailMessage>,
    pub msg_sel: usize,
    /// `@odata.nextLink` for the current folder listing (Some = more to load).
    pub messages_next: Option<String>,
    /// Guards against firing multiple "load more" requests at once.
    pub loading_more: bool,
    pub reading: Option<MailMessage>,
    pub calendar: Vec<CalEvent>,
}

pub struct TeamsState {
    pub mode: TeamsMode,
    pub chats: Vec<Chat>,
    pub chat_sel: usize,
    pub teams: Vec<Team>,
    pub team_sel: usize,
    pub channels: Vec<m365_core::models::Channel>,
    pub channel_sel: usize,
    pub messages: Vec<ChatMessage>,
    pub open_chat_id: Option<String>,
    pub open_channel: Option<(String, String)>,
    pub composer: String,
    pub focus: TeamsFocus,
}

impl Default for TeamsState {
    fn default() -> Self {
        Self {
            mode: TeamsMode::Chats,
            chats: Vec::new(),
            chat_sel: 0,
            teams: Vec::new(),
            team_sel: 0,
            channels: Vec::new(),
            channel_sel: 0,
            messages: Vec::new(),
            open_chat_id: None,
            open_channel: None,
            composer: String::new(),
            focus: TeamsFocus::List,
        }
    }
}

pub struct App {
    pub session: Session,
    pub tx: mpsc::Sender<AppMessage>,
    pub screen: Screen,
    pub outlook: OutlookState,
    pub outlook_focus: OutlookFocus,
    pub teams: TeamsState,
    pub overlay: Option<Overlay>,
    pub status: String,
    pub me: Option<User>,
    pub should_quit: bool,
}

/// Palette command identifiers.
const PALETTE_COMMANDS: &[(&str, &str)] = &[
    ("outlook", "Switch to Outlook"),
    ("teams", "Switch to Teams"),
    ("compose", "Compose new mail"),
    ("calendar", "Open calendar (today)"),
    ("chat-sender", "Teams: chat with selected email's sender"),
    ("refresh", "Refresh current view"),
    ("help", "Show help"),
    ("quit", "Quit"),
];

impl App {
    pub fn new(session: Session, tx: mpsc::Sender<AppMessage>) -> Self {
        Self {
            session,
            tx,
            screen: Screen::Outlook,
            outlook: OutlookState::default(),
            outlook_focus: OutlookFocus::Messages,
            teams: TeamsState::default(),
            overlay: None,
            status: "loading…".into(),
            me: None,
            should_quit: false,
        }
    }

    /// Kick off the initial data loads.
    pub fn bootstrap(&mut self) {
        self.load_whoami();
        self.load_folders();
        self.load_chats();
    }

    // -- background task helpers ------------------------------------------

    fn spawn<F>(&self, fut: F)
    where
        F: Future<Output = anyhow::Result<AppMessage>> + Send + 'static,
    {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let msg = match fut.await {
                Ok(m) => m,
                Err(e) => AppMessage::Error(format!("{e:#}")),
            };
            let _ = tx.send(msg).await;
        });
    }

    fn load_whoami(&self) {
        let s = self.session.clone();
        self.spawn(async move { Ok(AppMessage::Whoami(s.whoami().await?)) });
    }

    fn load_folders(&self) {
        let s = self.session.clone();
        self.spawn(async move { Ok(AppMessage::Folders(mail::list_folders(&s.graph).await?)) });
    }

    fn load_messages(&self, folder_id: String) {
        let s = self.session.clone();
        self.spawn(async move {
            let (items, next) = mail::list_messages(&s.graph, &folder_id, 50).await?;
            Ok(AppMessage::Messages {
                items,
                next,
                append: false,
            })
        });
    }

    fn load_more_messages(&mut self) {
        if self.outlook.loading_more {
            return;
        }
        let Some(next_link) = self.outlook.messages_next.clone() else {
            return;
        };
        self.outlook.loading_more = true;
        self.status = "loading more…".into();
        let s = self.session.clone();
        self.spawn(async move {
            let (items, next) = mail::list_messages_more(&s.graph, &next_link).await?;
            Ok(AppMessage::Messages {
                items,
                next,
                append: true,
            })
        });
    }

    fn load_body(&self, id: String) {
        let s = self.session.clone();
        self.spawn(async move { Ok(AppMessage::MessageBody(mail::get_message(&s.graph, &id).await?)) });
    }

    fn load_calendar(&self) {
        let s = self.session.clone();
        // Today .. +7 days in UTC.
        let start = chrono::Utc::now();
        let end = start + chrono::Duration::days(7);
        let start = start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let end = end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.spawn(async move {
            Ok(AppMessage::Calendar(
                calendar::calendar_view(&s.graph, &start, &end).await?,
            ))
        });
    }

    fn load_chats(&self) {
        let s = self.session.clone();
        self.spawn(async move { Ok(AppMessage::Chats(chats::list_chats(&s.graph, 40).await?)) });
    }

    fn load_chat_messages(&self, chat_id: String) {
        let s = self.session.clone();
        self.spawn(async move {
            let messages = chats::list_messages(&s.graph, &chat_id, 40).await?;
            Ok(AppMessage::ChatMessages { chat_id, messages })
        });
    }

    fn load_teams(&self) {
        let s = self.session.clone();
        self.spawn(async move { Ok(AppMessage::Teams(channels::joined_teams(&s.graph).await?)) });
    }

    fn load_channels(&self, team_id: String) {
        let s = self.session.clone();
        self.spawn(async move {
            let channels = channels::list_channels(&s.graph, &team_id).await?;
            Ok(AppMessage::Channels { team_id, channels })
        });
    }

    fn load_channel_messages(&self, team_id: String, channel_id: String) {
        let s = self.session.clone();
        self.spawn(async move {
            let messages = channels::list_messages(&s.graph, &team_id, &channel_id, 40).await?;
            Ok(AppMessage::ChannelMessages {
                team_id,
                channel_id,
                messages,
            })
        });
    }

    fn send_chat_message(&self, chat_id: String, text: String) {
        let s = self.session.clone();
        self.spawn(async move {
            chats::send_message(&s.graph, &chat_id, &text).await?;
            Ok(AppMessage::Done("message sent".into()))
        });
    }

    fn send_channel_message(&self, team_id: String, channel_id: String, text: String) {
        let s = self.session.clone();
        self.spawn(async move {
            channels::send_message(&s.graph, &team_id, &channel_id, &text).await?;
            Ok(AppMessage::Done("message posted".into()))
        });
    }

    fn send_mail(&self, to: Vec<String>, subject: String, body: String) {
        let s = self.session.clone();
        self.spawn(async move {
            mail::send_mail(&s.graph, &to, &subject, &body).await?;
            Ok(AppMessage::Done("mail sent".into()))
        });
    }

    fn reply_mail(&self, id: String, comment: String) {
        let s = self.session.clone();
        self.spawn(async move {
            mail::reply(&s.graph, &id, &comment).await?;
            Ok(AppMessage::Done("reply sent".into()))
        });
    }

    fn open_chat_with_email(&self, email: String) {
        let s = self.session.clone();
        let me_id = self.me.as_ref().map(|m| m.id.clone());
        self.spawn(async move {
            let Some(me_id) = me_id else {
                return Ok(AppMessage::Error("still loading your profile".into()));
            };
            let id = navigation::chat_id_for_email(&s, &me_id, &email).await?;
            Ok(AppMessage::OpenChat(id))
        });
    }

    // -- applying background results --------------------------------------

    pub fn apply(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::Status(s) => self.status = s,
            AppMessage::Error(e) => self.status = format!("error: {e}"),
            AppMessage::Whoami(u) => {
                self.status = format!(
                    "signed in as {} <{}>",
                    u.display_name.clone().unwrap_or_default(),
                    u.best_email().unwrap_or("")
                );
                self.me = Some(u);
            }
            AppMessage::Folders(f) => {
                self.outlook.folders = f;
                // Prefer Inbox as the initial selection.
                if let Some(i) = self
                    .outlook
                    .folders
                    .iter()
                    .position(|f| f.display_name.as_deref() == Some("Inbox"))
                {
                    self.outlook.folder_sel = i;
                }
                if let Some(f) = self.outlook.folders.get(self.outlook.folder_sel) {
                    self.load_messages(f.id.clone());
                }
            }
            AppMessage::Messages {
                items,
                next,
                append,
            } => {
                if append {
                    self.outlook.messages.extend(items);
                    self.outlook.loading_more = false;
                    self.status = if next.is_some() {
                        format!("{} loaded (more available)", self.outlook.messages.len())
                    } else {
                        format!("{} loaded (all)", self.outlook.messages.len())
                    };
                } else {
                    self.outlook.messages = items;
                    self.outlook.msg_sel = 0;
                    self.outlook.loading_more = false;
                }
                self.outlook.messages_next = next;
            }
            AppMessage::MessageBody(m) => self.outlook.reading = Some(m),
            AppMessage::Calendar(e) => self.outlook.calendar = e,
            AppMessage::Chats(c) => {
                self.teams.chats = c;
                self.teams.chat_sel = self.teams.chat_sel.min(self.teams.chats.len().saturating_sub(1));
            }
            AppMessage::ChatMessages { chat_id, messages } => {
                if self.teams.open_chat_id.as_deref() == Some(&chat_id) {
                    self.teams.messages = messages;
                }
            }
            AppMessage::Teams(t) => self.teams.teams = t,
            AppMessage::Channels { team_id, channels } => {
                if self.teams.teams.get(self.teams.team_sel).map(|t| &t.id) == Some(&team_id) {
                    self.teams.channels = channels;
                    self.teams.channel_sel = 0;
                }
            }
            AppMessage::ChannelMessages {
                team_id,
                channel_id,
                messages,
            } => {
                if self.teams.open_channel.as_ref() == Some(&(team_id, channel_id)) {
                    self.teams.messages = messages;
                }
            }
            AppMessage::Done(s) => {
                self.status = s;
                self.refresh_current();
            }
            AppMessage::OpenChat(Some(id)) => {
                self.screen = Screen::Teams;
                self.teams.mode = TeamsMode::Chats;
                self.teams.open_chat_id = Some(id.clone());
                self.teams.focus = TeamsFocus::Messages;
                self.load_chat_messages(id);
                self.load_chats();
            }
            AppMessage::OpenChat(None) => {
                self.status = "that sender isn't a Teams user in your directory".into();
            }
        }
    }

    pub fn on_change(&mut self, change: ChangeEvent) {
        match change.kind() {
            ChangeKind::Mail => {
                self.status = "📬 new mail".into();
                if let Some(f) = self.outlook.folders.get(self.outlook.folder_sel) {
                    self.load_messages(f.id.clone());
                }
            }
            ChangeKind::Chat => {
                self.status = "💬 chat update".into();
                self.load_chats();
                if let Some(id) = self.teams.open_chat_id.clone() {
                    self.load_chat_messages(id);
                }
            }
            ChangeKind::Channel => {
                if let Some((t, c)) = self.teams.open_channel.clone() {
                    self.load_channel_messages(t, c);
                }
            }
            ChangeKind::Other => {}
        }
    }

    fn refresh_current(&mut self) {
        match self.screen {
            Screen::Outlook => {
                if let Some(f) = self.outlook.folders.get(self.outlook.folder_sel) {
                    self.load_messages(f.id.clone());
                }
            }
            Screen::Teams => match self.teams.mode {
                TeamsMode::Chats => {
                    self.load_chats();
                    if let Some(id) = self.teams.open_chat_id.clone() {
                        self.load_chat_messages(id);
                    }
                }
                TeamsMode::Channels => {
                    if let Some((t, c)) = self.teams.open_channel.clone() {
                        self.load_channel_messages(t, c);
                    }
                }
            },
        }
    }

    // -- key handling ------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        // Overlays capture input first.
        if self.overlay.is_some() {
            self.on_key_overlay(key);
            return;
        }

        // Global bindings.
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::NONE) => {
                self.should_quit = true;
                return;
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return;
            }
            (KeyCode::F(2), _) => {
                self.screen = match self.screen {
                    Screen::Outlook => Screen::Teams,
                    Screen::Teams => Screen::Outlook,
                };
                return;
            }
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                self.overlay = Some(Overlay::Palette {
                    query: String::new(),
                    sel: 0,
                });
                return;
            }
            (KeyCode::Char('?'), _) => {
                self.overlay = Some(Overlay::Help);
                return;
            }
            _ => {}
        }

        match self.screen {
            Screen::Outlook => self.on_key_outlook(key),
            Screen::Teams => self.on_key_teams(key),
        }
    }

    fn on_key_outlook(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => {
                self.outlook_focus = match self.outlook_focus {
                    OutlookFocus::Folders => OutlookFocus::Messages,
                    OutlookFocus::Messages => OutlookFocus::Reading,
                    OutlookFocus::Reading => OutlookFocus::Folders,
                };
            }
            KeyCode::Char('g') => self.load_calendar_and_show(),
            KeyCode::Char('c') => {
                self.overlay = Some(Overlay::Compose(Compose {
                    kind: ComposeKind::NewMail,
                    to: String::new(),
                    subject: String::new(),
                    body: String::new(),
                    field: 0,
                }));
            }
            KeyCode::Char('/') => {
                self.overlay = Some(Overlay::Search { query: String::new() });
            }
            KeyCode::Char('r') => {
                if let Some(m) = self.current_mail() {
                    self.overlay = Some(Overlay::Compose(Compose {
                        kind: ComposeKind::ReplyMail { id: m.id.clone() },
                        to: String::new(),
                        subject: String::new(),
                        body: String::new(),
                        field: 2,
                    }));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.outlook_move(-1),
            KeyCode::Down | KeyCode::Char('j') => self.outlook_move(1),
            KeyCode::Enter => self.outlook_enter(),
            _ => {}
        }
    }

    fn load_calendar_and_show(&mut self) {
        self.load_calendar();
        self.overlay = Some(Overlay::Calendar);
    }

    fn outlook_move(&mut self, delta: i32) {
        match self.outlook_focus {
            OutlookFocus::Folders => {
                self.outlook.folder_sel =
                    step(self.outlook.folder_sel, delta, self.outlook.folders.len());
            }
            OutlookFocus::Messages | OutlookFocus::Reading => {
                let len = self.outlook.messages.len();
                self.outlook.msg_sel = step(self.outlook.msg_sel, delta, len);
                // Scrolling down onto the last row pulls the next page.
                if delta > 0 && len > 0 && self.outlook.msg_sel == len - 1 {
                    self.load_more_messages();
                }
            }
        }
    }

    fn outlook_enter(&mut self) {
        match self.outlook_focus {
            OutlookFocus::Folders => {
                if let Some(f) = self.outlook.folders.get(self.outlook.folder_sel) {
                    self.load_messages(f.id.clone());
                    self.outlook_focus = OutlookFocus::Messages;
                }
            }
            OutlookFocus::Messages | OutlookFocus::Reading => {
                if let Some(m) = self.current_mail() {
                    let id = m.id.clone();
                    self.load_body(id);
                    self.outlook_focus = OutlookFocus::Reading;
                }
            }
        }
    }

    fn current_mail(&self) -> Option<&MailMessage> {
        self.outlook.messages.get(self.outlook.msg_sel)
    }

    fn on_key_teams(&mut self, key: KeyEvent) {
        // Composer captures typing when focused.
        if self.teams.focus == TeamsFocus::Composer {
            match key.code {
                KeyCode::Esc => self.teams.focus = TeamsFocus::Messages,
                KeyCode::Tab => self.teams.focus = TeamsFocus::List,
                KeyCode::Enter => self.teams_send(),
                KeyCode::Backspace => {
                    self.teams.composer.pop();
                }
                KeyCode::Char(c) => self.teams.composer.push(c),
                _ => {}
            }
            return;
        }

        match key.code {
            // Back out to the conversation list from the messages pane.
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
                self.teams.focus = TeamsFocus::List;
            }
            KeyCode::Tab => {
                self.teams.focus = match self.teams.focus {
                    TeamsFocus::List => TeamsFocus::Messages,
                    TeamsFocus::Messages => TeamsFocus::Composer,
                    TeamsFocus::Composer => TeamsFocus::List,
                };
            }
            KeyCode::Char('t') => {
                // Toggle chats/channels mode.
                self.teams.mode = match self.teams.mode {
                    TeamsMode::Chats => {
                        if self.teams.teams.is_empty() {
                            self.load_teams();
                        }
                        TeamsMode::Channels
                    }
                    TeamsMode::Channels => TeamsMode::Chats,
                };
            }
            KeyCode::Char('i') => self.teams.focus = TeamsFocus::Composer,
            KeyCode::Up | KeyCode::Char('k') => self.teams_move(-1),
            KeyCode::Down | KeyCode::Char('j') => self.teams_move(1),
            KeyCode::Enter => self.teams_enter(),
            _ => {}
        }
    }

    fn teams_move(&mut self, delta: i32) {
        match (self.teams.mode, self.teams.focus) {
            (TeamsMode::Chats, TeamsFocus::List) => {
                self.teams.chat_sel = step(self.teams.chat_sel, delta, self.teams.chats.len());
            }
            (TeamsMode::Channels, TeamsFocus::List) => {
                // Navigate channels; if none loaded, navigate teams.
                if self.teams.channels.is_empty() {
                    self.teams.team_sel = step(self.teams.team_sel, delta, self.teams.teams.len());
                } else {
                    self.teams.channel_sel =
                        step(self.teams.channel_sel, delta, self.teams.channels.len());
                }
            }
            _ => {}
        }
    }

    fn teams_enter(&mut self) {
        match self.teams.mode {
            TeamsMode::Chats => {
                if let Some(c) = self.teams.chats.get(self.teams.chat_sel) {
                    let id = c.id.clone();
                    self.teams.open_chat_id = Some(id.clone());
                    self.teams.open_channel = None;
                    self.teams.messages.clear();
                    self.load_chat_messages(id);
                    self.teams.focus = TeamsFocus::Messages;
                }
            }
            TeamsMode::Channels => {
                if self.teams.channels.is_empty() {
                    if let Some(t) = self.teams.teams.get(self.teams.team_sel) {
                        self.load_channels(t.id.clone());
                    }
                } else if let (Some(t), Some(ch)) = (
                    self.teams.teams.get(self.teams.team_sel),
                    self.teams.channels.get(self.teams.channel_sel),
                ) {
                    let team_id = t.id.clone();
                    let ch_id = ch.id.clone();
                    self.teams.open_channel = Some((team_id.clone(), ch_id.clone()));
                    self.teams.open_chat_id = None;
                    self.teams.messages.clear();
                    self.load_channel_messages(team_id, ch_id);
                    self.teams.focus = TeamsFocus::Messages;
                }
            }
        }
    }

    fn teams_send(&mut self) {
        let text = self.teams.composer.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.teams.composer.clear();
        match self.teams.mode {
            TeamsMode::Chats => {
                if let Some(id) = self.teams.open_chat_id.clone() {
                    self.send_chat_message(id, text);
                }
            }
            TeamsMode::Channels => {
                if let Some((t, c)) = self.teams.open_channel.clone() {
                    self.send_channel_message(t, c, text);
                }
            }
        }
    }

    // -- overlay input -----------------------------------------------------

    fn on_key_overlay(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.overlay = None;
            return;
        }
        // Take the overlay out so we can mutate self freely, then put it back.
        let overlay = self.overlay.take();
        match overlay {
            Some(Overlay::Help) | Some(Overlay::Calendar) => {
                // any key besides Esc closes
            }
            Some(Overlay::Search { mut query }) => match key.code {
                KeyCode::Enter => {
                    let q = query.clone();
                    self.run_search(q);
                }
                KeyCode::Backspace => {
                    query.pop();
                    self.overlay = Some(Overlay::Search { query });
                }
                KeyCode::Char(c) => {
                    query.push(c);
                    self.overlay = Some(Overlay::Search { query });
                }
                _ => self.overlay = Some(Overlay::Search { query }),
            },
            Some(Overlay::Palette { mut query, mut sel }) => {
                let matches = filter_commands(&query);
                match key.code {
                    KeyCode::Enter => {
                        if let Some((id, _)) = matches.get(sel) {
                            self.run_command(id);
                        }
                    }
                    KeyCode::Up => {
                        sel = sel.saturating_sub(1);
                        self.overlay = Some(Overlay::Palette { query, sel });
                    }
                    KeyCode::Down => {
                        if sel + 1 < matches.len() {
                            sel += 1;
                        }
                        self.overlay = Some(Overlay::Palette { query, sel });
                    }
                    KeyCode::Backspace => {
                        query.pop();
                        self.overlay = Some(Overlay::Palette { query, sel: 0 });
                    }
                    KeyCode::Char(c) => {
                        query.push(c);
                        self.overlay = Some(Overlay::Palette { query, sel: 0 });
                    }
                    _ => self.overlay = Some(Overlay::Palette { query, sel }),
                }
            }
            Some(Overlay::Compose(mut c)) => self.on_key_compose(key, &mut c),
            None => {}
        }
    }

    fn on_key_compose(&mut self, key: KeyEvent, c: &mut Compose) {
        let is_reply = matches!(c.kind, ComposeKind::ReplyMail { .. });
        match key.code {
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.submit_compose(c);
                return;
            }
            KeyCode::Tab => {
                // Replies only have the body field.
                if is_reply {
                    c.field = 2;
                } else {
                    c.field = (c.field + 1) % 3;
                }
            }
            KeyCode::Backspace => {
                field_mut(c).pop();
            }
            KeyCode::Enter => {
                if c.field == 2 {
                    field_mut(c).push('\n');
                } else {
                    c.field = (c.field + 1) % 3;
                }
            }
            KeyCode::Char(ch) => field_mut(c).push(ch),
            _ => {}
        }
        // Put the (mutated) compose overlay back.
        self.overlay = Some(Overlay::Compose(std::mem::replace(
            c,
            Compose {
                kind: ComposeKind::NewMail,
                to: String::new(),
                subject: String::new(),
                body: String::new(),
                field: 0,
            },
        )));
    }

    fn submit_compose(&mut self, c: &mut Compose) {
        match &c.kind {
            ComposeKind::NewMail => {
                let to: Vec<String> = c
                    .to
                    .split([',', ';', ' '])
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
                if to.is_empty() {
                    self.status = "add at least one recipient".into();
                    self.overlay = Some(Overlay::Compose(std::mem::replace(
                        c,
                        empty_compose(),
                    )));
                    return;
                }
                self.send_mail(to, c.subject.clone(), c.body.clone());
            }
            ComposeKind::ReplyMail { id } => {
                self.reply_mail(id.clone(), c.body.clone());
            }
        }
        self.overlay = None;
    }

    fn run_search(&mut self, query: String) {
        self.overlay = None;
        if query.trim().is_empty() {
            return;
        }
        let s = self.session.clone();
        self.status = format!("searching \"{query}\"…");
        self.spawn(async move {
            let results = mail::search(&s.graph, &query, 50).await?;
            Ok(AppMessage::Messages {
                items: results,
                next: None,
                append: false,
            })
        });
    }

    fn run_command(&mut self, id: &str) {
        self.overlay = None;
        match id {
            "outlook" => self.screen = Screen::Outlook,
            "teams" => {
                self.screen = Screen::Teams;
                if self.teams.chats.is_empty() {
                    self.load_chats();
                }
            }
            "compose" => {
                self.overlay = Some(Overlay::Compose(empty_compose()));
            }
            "calendar" => self.load_calendar_and_show(),
            "chat-sender" => {
                if let Some(addr) = self.current_mail().and_then(|m| m.sender_address()) {
                    self.status = format!("opening chat with {addr}…");
                    self.open_chat_with_email(addr);
                } else {
                    self.status = "select an email first".into();
                }
            }
            "refresh" => self.refresh_current(),
            "help" => self.overlay = Some(Overlay::Help),
            "quit" => self.should_quit = true,
            _ => {}
        }
    }
}

fn empty_compose() -> Compose {
    Compose {
        kind: ComposeKind::NewMail,
        to: String::new(),
        subject: String::new(),
        body: String::new(),
        field: 0,
    }
}

fn field_mut(c: &mut Compose) -> &mut String {
    match c.field {
        0 => &mut c.to,
        1 => &mut c.subject,
        _ => &mut c.body,
    }
}

/// Move a selection index by `delta`, clamped to `[0, len)`.
fn step(idx: usize, delta: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let max = len - 1;
    if delta < 0 {
        idx.saturating_sub((-delta) as usize)
    } else {
        (idx + delta as usize).min(max)
    }
}

/// Palette fuzzy-ish filter (case-insensitive substring on label or id).
pub fn filter_commands(query: &str) -> Vec<(&'static str, &'static str)> {
    let q = query.to_ascii_lowercase();
    PALETTE_COMMANDS
        .iter()
        .filter(|(id, label)| {
            q.is_empty()
                || label.to_ascii_lowercase().contains(&q)
                || id.contains(&q)
        })
        .copied()
        .collect()
}
