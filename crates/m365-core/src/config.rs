//! Runtime configuration, loaded from the environment (and an optional `.env`).

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Microsoft Graph base URL (v1.0 endpoint).
pub const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

/// The delegated scopes the app requests. `offline_access` is required for
/// refresh tokens; `openid`/`profile` give us the signed-in user's identity.
pub const DEFAULT_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "offline_access",
    "User.Read",
    "People.Read",
    "Mail.ReadWrite",
    "Mail.Send",
    "Calendars.ReadWrite",
    "Chat.ReadWrite",
    "ChannelMessage.Send",
    "ChannelMessage.Read.All",
    "Presence.Read.All",
];

#[derive(Debug, Clone)]
pub struct Config {
    /// Entra application (client) ID of the registered public client.
    pub client_id: String,
    /// Directory tenant ID (or `organizations` / `common`).
    pub tenant_id: String,
    /// Delegated scopes to request.
    pub scopes: Vec<String>,
    /// Public HTTPS base of the tunnel that fronts the webhook service, e.g.
    /// `https://m365.example.com`. `None` disables push (poll-only mode).
    pub tunnel_base_url: Option<String>,
    /// Redis connection URL used to receive change events from the webhook.
    pub redis_url: String,
    /// Path of the on-disk token cache (0600).
    pub token_cache_path: PathBuf,
    /// Shared secret echoed in subscription `clientState` and verified by the
    /// webhook. Generated on first run if absent.
    pub client_state: String,
}

impl Config {
    /// Load configuration from the process environment. Call
    /// [`Config::load_dotenv`] first if you want `.env` support.
    pub fn from_env() -> Result<Self> {
        let client_id = env_required("M365_CLIENT_ID")?;
        let tenant_id = std::env::var("M365_TENANT_ID").unwrap_or_else(|_| "organizations".into());

        let scopes = match std::env::var("M365_SCOPES") {
            Ok(s) if !s.trim().is_empty() => {
                s.split_whitespace().map(|s| s.to_string()).collect()
            }
            _ => DEFAULT_SCOPES.iter().map(|s| s.to_string()).collect(),
        };

        let tunnel_base_url = std::env::var("M365_TUNNEL_BASE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim_end_matches('/').to_string());

        let redis_url =
            std::env::var("M365_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());

        let token_cache_path = match std::env::var("M365_TOKEN_CACHE") {
            Ok(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => default_cache_dir()?.join("token-cache.json"),
        };

        let client_state = std::env::var("M365_CLIENT_STATE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        Ok(Self {
            client_id,
            tenant_id,
            scopes,
            tunnel_base_url,
            redis_url,
            token_cache_path,
            client_state,
        })
    }

    /// Best-effort load of a `.env` file from the current directory or nearest
    /// parent. Missing file is not an error.
    pub fn load_dotenv() {
        let _ = dotenvy::dotenv();
    }

    pub fn scope_string(&self) -> String {
        self.scopes.join(" ")
    }

    pub fn devicecode_endpoint(&self) -> String {
        format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/devicecode",
            self.tenant_id
        )
    }

    pub fn token_endpoint(&self) -> String {
        format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.tenant_id
        )
    }

    /// URL the TUI points its subscriptions at (`None` if no tunnel configured).
    pub fn notification_url(&self) -> Option<String> {
        self.tunnel_base_url
            .as_ref()
            .map(|b| format!("{b}/notifications"))
    }

    pub fn lifecycle_url(&self) -> Option<String> {
        self.tunnel_base_url
            .as_ref()
            .map(|b| format!("{b}/lifecycle"))
    }
}

fn env_required(key: &str) -> Result<String> {
    std::env::var(key)
        .with_context(|| format!("required environment variable {key} is not set"))
        .map(|s| s.trim().to_string())
        .and_then(|s| {
            if s.is_empty() {
                anyhow::bail!("environment variable {key} is empty");
            }
            Ok(s)
        })
}

fn default_cache_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("pt", "contoso", "m365-tui")
        .context("could not determine a config directory for this platform")?;
    let dir = dirs.config_dir().to_path_buf();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating config dir {}", dir.display()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tunnel: Option<&str>) -> Config {
        Config {
            client_id: "cid".into(),
            tenant_id: "organizations".into(),
            scopes: vec!["User.Read".into(), "Mail.Send".into()],
            tunnel_base_url: tunnel.map(|s| s.to_string()),
            redis_url: "redis://127.0.0.1:6379".into(),
            token_cache_path: PathBuf::from("/tmp/x.json"),
            client_state: "secret".into(),
        }
    }

    #[test]
    fn builds_endpoints_and_urls() {
        let c = sample(Some("https://m365.example.com"));
        assert_eq!(c.scope_string(), "User.Read Mail.Send");
        assert_eq!(
            c.token_endpoint(),
            "https://login.microsoftonline.com/organizations/oauth2/v2.0/token"
        );
        assert_eq!(
            c.notification_url().as_deref(),
            Some("https://m365.example.com/notifications")
        );
        assert_eq!(
            c.lifecycle_url().as_deref(),
            Some("https://m365.example.com/lifecycle")
        );
    }

    #[test]
    fn no_tunnel_means_no_notification_url() {
        let c = sample(None);
        assert!(c.notification_url().is_none());
    }
}
