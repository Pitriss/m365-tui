//! Local event bus: subscribe to the Redis channel the webhook publishes change
//! notifications on, and forward typed events to the TUI over an mpsc channel.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Redis pub/sub channel shared with the webhook service.
pub const CHANGES_CHANNEL: &str = "m365:changes";

/// A normalized change signal (no resource data — just "this changed").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeEvent {
    /// Graph resource path that changed, e.g. `chats('19:...')/messages('...')`.
    pub resource: String,
    /// `created`, `updated`, or `deleted`.
    #[serde(default)]
    pub change_type: String,
    #[serde(default)]
    pub subscription_id: Option<String>,
    /// Present on lifecycle events (`reauthorizationRequired`, etc.).
    #[serde(default)]
    pub lifecycle_event: Option<String>,
}

impl ChangeEvent {
    /// Coarse routing hint for the TUI.
    pub fn kind(&self) -> ChangeKind {
        let r = self.resource.to_ascii_lowercase();
        if r.contains("mailfolders") || r.contains("/messages") && r.contains("mail") {
            ChangeKind::Mail
        } else if r.contains("chats") {
            ChangeKind::Chat
        } else if r.contains("channels") || r.contains("teams") {
            ChangeKind::Channel
        } else if r.starts_with("me/messages") || r.contains("mailfolders") {
            ChangeKind::Mail
        } else {
            ChangeKind::Other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Mail,
    Chat,
    Channel,
    Other,
}

/// Connect to Redis and forward every published [`ChangeEvent`] to `tx` until
/// the connection drops or the receiver is closed. Callers typically run this
/// in a background task with reconnection.
pub async fn run_subscriber(redis_url: &str, tx: mpsc::Sender<ChangeEvent>) -> Result<()> {
    let client = redis::Client::open(redis_url).context("opening Redis client")?;
    let mut pubsub = client
        .get_async_pubsub()
        .await
        .context("connecting Redis pubsub")?;
    pubsub
        .subscribe(CHANGES_CHANNEL)
        .await
        .context("subscribing to changes channel")?;

    let mut stream = pubsub.on_message();
    while let Some(msg) = stream.next().await {
        let payload: String = match msg.get_payload() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("bad Redis payload: {e}");
                continue;
            }
        };
        match serde_json::from_str::<ChangeEvent>(&payload) {
            Ok(ev) => {
                if tx.send(ev).await.is_err() {
                    break; // receiver gone
                }
            }
            Err(e) => tracing::warn!("undecodable change event: {e}: {payload}"),
        }
    }
    Ok(())
}

/// Reconnecting wrapper: retries the subscriber forever with backoff. Intended
/// to be spawned as a background task.
pub async fn run_subscriber_forever(redis_url: String, tx: mpsc::Sender<ChangeEvent>) {
    let mut delay = std::time::Duration::from_secs(1);
    loop {
        match run_subscriber(&redis_url, tx.clone()).await {
            Ok(()) => {
                tracing::info!("Redis subscriber ended; reconnecting");
            }
            Err(e) => {
                tracing::warn!("Redis subscriber error: {e:#}; retrying in {:?}", delay);
            }
        }
        if tx.is_closed() {
            break;
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(std::time::Duration::from_secs(30));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_event_round_trips() {
        let ev = ChangeEvent {
            resource: "chats('19:abc')/messages('123')".into(),
            change_type: "created".into(),
            subscription_id: Some("sub-1".into()),
            lifecycle_event: None,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: ChangeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.resource, ev.resource);
        assert_eq!(back.kind(), ChangeKind::Chat);
    }

    #[test]
    fn routing_classifies_resources() {
        let mail = ChangeEvent {
            resource: "me/mailFolders('inbox')/messages('AAA')".into(),
            change_type: "created".into(),
            subscription_id: None,
            lifecycle_event: None,
        };
        assert_eq!(mail.kind(), ChangeKind::Mail);

        let chat = ChangeEvent {
            resource: "chats('19:xyz')/messages('1')".into(),
            change_type: "updated".into(),
            subscription_id: None,
            lifecycle_event: None,
        };
        assert_eq!(chat.kind(), ChangeKind::Chat);
    }
}
