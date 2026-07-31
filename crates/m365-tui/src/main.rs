//! m365 — a unified terminal client for Outlook and Microsoft Teams.
//!
//! Usage:
//!   m365            launch the TUI
//!   m365 whoami     print the signed-in user and exit (auth smoke test)
//!   m365 login      run device-code login and exit

mod app;
mod clipboard;
mod content;
mod editor;
mod files;
mod navigation;
mod opener;
mod ui;

use std::io::stdout;
use std::time::Duration;

use anyhow::{Context, Result};
use app::{App, AppMessage};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures_util::StreamExt;
use m365_core::events::ChangeEvent;
use m365_core::{subscriptions, DeviceCodePrompt, Session};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

/// How often to poll the server to refresh the current view.
const POLL_SECONDS: u64 = 20;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let session = match Session::from_env() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("configuration error: {e:#}");
            eprintln!("\nSet at least M365_CLIENT_ID (see README / .env.example).");
            std::process::exit(1);
        }
    };

    // Device-code login up front (prints the code to stdout, before the TUI).
    session
        .ensure_logged_in(|p: DeviceCodePrompt| print_device_prompt(&p))
        .await
        .context("sign-in failed")?;

    match std::env::args().nth(1).as_deref() {
        Some("whoami") => {
            let me = session.whoami().await?;
            println!(
                "{} <{}>",
                me.display_name.clone().unwrap_or_default(),
                me.best_email().unwrap_or("")
            );
            Ok(())
        }
        Some("login") => {
            println!("signed in — token cached.");
            Ok(())
        }
        _ => run_tui(session).await,
    }
}

fn print_device_prompt(p: &DeviceCodePrompt) {
    println!("\n──────────────────────────────────────────────");
    println!(" Sign in to Microsoft 365");
    println!(" 1. Open: {}", p.verification_uri);
    println!(" 2. Enter code: {}", p.user_code);
    println!("──────────────────────────────────────────────");
    println!(" {}", p.message);
    println!(" (waiting for you to finish in the browser…)\n");
}

async fn run_tui(session: Session) -> Result<()> {
    // Channels: background task results, and Graph change events.
    let (tx, mut app_rx) = mpsc::channel::<AppMessage>(256);
    let (change_tx, mut change_rx) = mpsc::channel::<ChangeEvent>(256);

    // Real-time push: only if a tunnel URL is configured.
    if session.config.notification_url().is_some() {
        spawn_realtime(&session, change_tx);
    } else {
        let _ = tx
            .send(AppMessage::Status(format!(
                "poll mode — refreshing every {POLL_SECONDS}s (set M365_TUNNEL_BASE_URL for instant push)"
            )))
            .await;
    }

    // Periodic poll so the current view refreshes from the server regardless of
    // whether push is configured.
    {
        let poll_tx = tx.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(POLL_SECONDS));
            ticker.tick().await; // the first tick fires immediately; skip it
            loop {
                ticker.tick().await;
                if poll_tx.send(AppMessage::Poll).await.is_err() {
                    break;
                }
            }
        });
    }

    let mut app = App::new(session, tx);
    app.bootstrap();

    // Terminal setup.
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;

    let res = event_loop(&mut terminal, &mut app, &mut app_rx, &mut change_rx).await;

    // Terminal teardown (best-effort even on error).
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), DisableBracketedPaste, LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    res
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    app_rx: &mut mpsc::Receiver<AppMessage>,
    change_rx: &mut mpsc::Receiver<ChangeEvent>,
) -> Result<()> {
    let mut reader = EventStream::new();
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        tokio::select! {
            maybe_event = reader.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(k))) if k.kind == KeyEventKind::Press => app.on_key(k),
                    Some(Ok(Event::Paste(text))) => app.on_paste(text),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                }
            }
            Some(msg) = app_rx.recv() => app.apply(msg),
            Some(change) = change_rx.recv() => app.on_change(change),
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// Subscribe to the webhook's Redis channel and keep Graph subscriptions alive.
fn spawn_realtime(session: &Session, change_tx: mpsc::Sender<ChangeEvent>) {
    let redis_url = session.config.redis_url.clone();
    tokio::spawn(m365_core::events::run_subscriber_forever(redis_url, change_tx));

    let session = session.clone();
    tokio::spawn(async move {
        if let Err(e) = manage_subscriptions(session).await {
            tracing::warn!("subscription manager stopped: {e:#}");
        }
    });
}

/// Create the inbox + all-chats subscriptions and renew them before they lapse.
/// Chat subscriptions expire in ~1h, so we renew every 45 minutes.
async fn manage_subscriptions(session: Session) -> Result<()> {
    let notify = session.config.notification_url().unwrap();
    let lifecycle = session.config.lifecycle_url();
    let state = &session.config.client_state;

    let mut ids: Vec<String> = Vec::new();
    let create = |res: &'static str| {
        let notify = notify.clone();
        let lifecycle = lifecycle.clone();
        let graph = session.graph.clone();
        let state = state.clone();
        async move {
            subscriptions::create(
                &graph,
                res,
                "created,updated",
                &notify,
                lifecycle.as_deref(),
                &state,
                55,
            )
            .await
        }
    };

    for res in [subscriptions::RES_INBOX, subscriptions::RES_ALL_CHATS] {
        match create(res).await {
            Ok(s) => {
                tracing::info!("subscribed to {res}: {}", s.id);
                ids.push(s.id);
            }
            Err(e) => tracing::warn!("failed to subscribe to {res}: {e:#}"),
        }
    }

    loop {
        tokio::time::sleep(Duration::from_secs(45 * 60)).await;
        for id in &ids {
            if let Err(e) = subscriptions::renew(&session.graph, id, 55).await {
                tracing::warn!("failed to renew subscription {id}: {e:#}");
            }
        }
    }
}

/// Log to a file in the cache dir so we never corrupt the TUI on stdout/stderr.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let log_path = std::env::temp_dir().join("m365-tui.log");
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
        .with_writer(move || {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap())
        })
        .try_init();
}
