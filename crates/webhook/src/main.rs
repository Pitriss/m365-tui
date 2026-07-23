//! Microsoft Graph change-notification receiver.
//!
//! Sits behind the Cloudflare tunnel. Responsibilities:
//!   * Echo the `validationToken` during subscription setup (the handshake).
//!   * Verify `clientState` on every notification.
//!   * Publish a small, resource-data-free [`ChangeEvent`] to Redis, which the
//!     TUI consumes and turns into a targeted delta fetch ("notify-then-delta").
//!
//! It holds no Graph token and never calls Graph — by design.

use std::net::SocketAddr;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use m365_core::events::{ChangeEvent, CHANGES_CHANNEL};
use redis::AsyncCommands;
use serde::Deserialize;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    redis: redis::aio::MultiplexedConnection,
    /// Expected `clientState`; notifications not matching are dropped.
    expected_state: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let redis_url =
        std::env::var("M365_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let expected_state = std::env::var("M365_CLIENT_STATE").ok().filter(|s| !s.is_empty());
    let bind: SocketAddr = std::env::var("WEBHOOK_BIND")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .expect("invalid WEBHOOK_BIND");

    let client = redis::Client::open(redis_url.as_str())?;
    let redis = client.get_multiplexed_async_connection().await?;

    if expected_state.is_none() {
        tracing::warn!("M365_CLIENT_STATE not set — clientState verification is DISABLED");
    }

    let state = AppState { redis, expected_state };

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/notifications", post(notifications))
        .route("/lifecycle", post(lifecycle))
        .with_state(state);

    tracing::info!("webhook listening on {bind}");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Deserialize)]
struct ValidationQuery {
    #[serde(rename = "validationToken")]
    validation_token: Option<String>,
}

/// Graph subscription payload envelope.
#[derive(Deserialize)]
struct NotificationBatch {
    #[serde(default)]
    value: Vec<Notification>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Notification {
    #[serde(default)]
    subscription_id: Option<String>,
    #[serde(default)]
    client_state: Option<String>,
    #[serde(default)]
    change_type: Option<String>,
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    lifecycle_event: Option<String>,
}

async fn notifications(
    Query(q): Query<ValidationQuery>,
    State(state): State<AppState>,
    body: String,
) -> Response {
    // Handshake: echo the validation token verbatim as text/plain.
    if let Some(token) = q.validation_token {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain")],
            token,
        )
            .into_response();
    }
    handle_batch(state, body, false).await
}

async fn lifecycle(
    Query(q): Query<ValidationQuery>,
    State(state): State<AppState>,
    body: String,
) -> Response {
    if let Some(token) = q.validation_token {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain")],
            token,
        )
            .into_response();
    }
    handle_batch(state, body, true).await
}

async fn handle_batch(mut state: AppState, body: String, is_lifecycle: bool) -> Response {
    let batch: NotificationBatch = match serde_json::from_str(&body) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("bad notification body: {e}");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    for n in batch.value {
        // Verify shared secret.
        if let Some(expected) = &state.expected_state {
            if n.client_state.as_deref() != Some(expected.as_str()) {
                tracing::warn!("dropping notification with mismatched clientState");
                continue;
            }
        }

        let event = ChangeEvent {
            resource: n.resource.unwrap_or_default(),
            change_type: n.change_type.unwrap_or_default(),
            subscription_id: n.subscription_id,
            lifecycle_event: if is_lifecycle { n.lifecycle_event } else { None },
        };

        match serde_json::to_string(&event) {
            Ok(payload) => {
                let res: redis::RedisResult<()> =
                    state.redis.publish(CHANGES_CHANNEL, payload).await;
                if let Err(e) = res {
                    tracing::error!("Redis publish failed: {e}");
                }
            }
            Err(e) => tracing::error!("serialize event failed: {e}"),
        }
    }

    // Graph expects a fast 2xx; 202 Accepted is conventional.
    StatusCode::ACCEPTED.into_response()
}
