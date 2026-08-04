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
mod notify;
mod opener;
mod ui;

use std::io::stdout;
use std::time::Duration;

use anyhow::{Context, Result};
use app::{App, AppMessage, PushState};
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

/// Lightweight local tick: refreshes memory usage and ages out status text.
/// Costs nothing on the network.
const TICK_SECONDS: u64 = 2;

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
        spawn_realtime(&session, change_tx, tx.clone());
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

    {
        let tick_tx = tx.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(TICK_SECONDS));
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if tick_tx.send(AppMessage::Tick).await.is_err() {
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

    // Drop our presence session on the way out, otherwise the user would keep
    // showing the status we published for up to the session lease.
    if app.presence_session.is_some() {
        let client_id = app.session.config.client_id.clone();
        if let Err(e) =
            m365_core::people::clear_session_presence(&app.session.graph, &client_id).await
        {
            tracing::warn!("could not clear presence session on exit: {e:#}");
        }
    }

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
fn spawn_realtime(
    session: &Session,
    change_tx: mpsc::Sender<ChangeEvent>,
    app_tx: mpsc::Sender<AppMessage>,
) {
    let redis_url = session.config.redis_url.clone();
    tokio::spawn(m365_core::events::run_subscriber_forever(redis_url, change_tx));

    let session = session.clone();
    tokio::spawn(async move {
        if let Err(e) = manage_subscriptions(session, app_tx.clone()).await {
            tracing::warn!("subscription manager stopped: {e:#}");
            let _ = app_tx
                .send(AppMessage::Push(PushState::Failed(format!("{e:#}"))))
                .await;
        }
    });
}

/// Create the inbox + all-chats subscriptions and renew them before they lapse.
/// Chat subscriptions expire in ~1h, so we renew every 45 minutes.
async fn manage_subscriptions(
    session: Session,
    app_tx: mpsc::Sender<AppMessage>,
) -> Result<()> {
    let _ = app_tx.send(AppMessage::Push(PushState::Connecting)).await;
    let notify = session.config.notification_url().unwrap();
    let lifecycle = session.config.lifecycle_url();
    let state = &session.config.client_state;

    let mut ids: Vec<String> = Vec::new();
    let create = |res: String| {
        let notify = notify.clone();
        let lifecycle = lifecycle.clone();
        let graph = session.graph.clone();
        let state = state.clone();
        async move {
            subscriptions::create(
                &graph,
                &res,
                "created,updated",
                &notify,
                lifecycle.as_deref(),
                &state,
                55,
            )
            .await
        }
    };

    // The chats resource needs the signed-in user's id spelled out.
    let resources: Vec<String> = match m365_core::people::me(&session.graph).await {
        Ok(user) => vec![
            subscriptions::RES_INBOX.to_string(),
            subscriptions::res_all_chats(&user.id),
        ],
        Err(e) => {
            tracing::warn!("could not resolve the signed-in user: {e:#}");
            vec![subscriptions::RES_INBOX.to_string()]
        }
    };

    let mut last_error = None;
    for res in resources {
        match create(res.clone()).await {
            Ok(s) => {
                tracing::info!("subscribed to {res}: {}", s.id);
                ids.push(s.id);
            }
            Err(e) => {
                tracing::warn!("failed to subscribe to {res}: {e:#}");
                last_error = Some(m365_core::util::graph_error_summary(&format!("{e:#}")));
            }
        }
    }

    // Report health so a broken tunnel is visible rather than silently
    // degrading to polling.
    let _ = app_tx
        .send(AppMessage::Push(if ids.is_empty() {
            PushState::Failed(last_error.unwrap_or_else(|| "no subscriptions created".into()))
        } else {
            PushState::Live
        }))
        .await;

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
