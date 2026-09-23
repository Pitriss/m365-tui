//! Runtime configuration, loaded from the environment (and an optional `.env`).

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Microsoft Graph base URL (v1.0 endpoint).
pub const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

/// The Graph endpoint to talk to, overridable with `M365_GRAPH_BASE`.
///
/// Pointing this at a local mock lets the app run on fabricated data — useful
/// for recording a demo without a real mailbox on screen, and for exercising the
/// UI offline. Unset in normal use.
pub fn graph_base() -> String {
    std::env::var("M365_GRAPH_BASE")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| GRAPH_BASE.to_string())
}

/// The delegated scopes the app requests. `offline_access` is required for
/// refresh tokens; `openid`/`profile` give us the signed-in user's identity.
///
/// Keep this list stable: changing it invalidates existing consent, and in
/// tenants that disallow user consent every sign-in then needs a fresh admin
/// approval. New optional capabilities should be opt-in (see
/// [`PRESENCE_WRITE_SCOPE`]) rather than added here.
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

/// Needed only to *set* your own presence (`setUserPreferredPresence`).
/// Opt in with `M365_PRESENCE_WRITE=1`; requires the permission to be added to
/// the app registration and consented.
pub const PRESENCE_WRITE_SCOPE: &str = "Presence.ReadWrite";

/// Needed to list the teams and channels you belong to (`/me/joinedTeams`).
/// Chats work without it. Opt in with `M365_TEAMS_CHANNELS=1`.
pub const TEAMS_READ_SCOPE: &str = "Team.ReadBasic.All";

/// Needed to download regular Teams file attachments from SharePoint/OneDrive.
/// Opt in with `M365_TEAMS_FILE_IMAGES=1`.
pub const FILES_READ_SCOPE: &str = "Files.Read.All";

/// Needed to read profile photos of other organization users.
/// Opt in with `M365_PROFILE_PHOTO=1`; kept out of DEFAULT_SCOPES so existing
/// consent is not disturbed for users who do not want contact avatars.
pub const PROFILE_PHOTO_SCOPE: &str = "User.ReadBasic.All";

/// Experimental full directory profile lookup for Teams contacts.
///
/// Opt in with `M365_DIRECTORY_PROFILE=1`. This adds delegated `User.Read.All`,
/// which requires administrator consent. The feature is currently untested.
pub const DIRECTORY_PROFILE_SCOPE: &str = "User.Read.All";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamsSystemEvents {
    Useful,
    All,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NtfyMode {
    Never,
    Always,
    Away,
    AlwaysWd,
    AwayWd,
}

impl NtfyMode {
    pub fn enabled(self) -> bool {
        self != Self::Never
    }

    pub fn needs_presence(self) -> bool {
        matches!(self, Self::Away | Self::AwayWd)
    }

    pub fn needs_work_plan(self) -> bool {
        matches!(self, Self::AlwaysWd | Self::AwayWd)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarNotify {
    All,
    Internal,
    External,
    None,
}

impl CalendarNotify {
    pub fn enabled(self) -> bool {
        self != Self::None
    }

    pub fn internal(self) -> bool {
        matches!(self, Self::All | Self::Internal)
    }

    pub fn external(self) -> bool {
        matches!(self, Self::All | Self::External)
    }
}

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
    /// Desktop notifications for direct messages and `@mentions`.
    pub notifications: bool,
    /// Forward notification events to ntfy.
    pub ntfy: NtfyMode,
    /// ntfy server root URL, e.g. https://ntfy.example.com.
    pub ntfy_server: Option<String>,
    /// ntfy topic to publish to.
    pub ntfy_topic: Option<String>,
    /// Optional ntfy Bearer token for protected topics.
    pub ntfy_token: Option<String>,
    /// Calendar reminder delivery mode: all, internal, external, or none.
    pub calendar_notify: CalendarNotify,
    /// Seconds an unread message must remain open before it is marked read.
    /// Zero marks it read immediately after the body is displayed.
    pub read_msg_timeout: u64,
    /// Show presence indicators for contacts in one-to-one Teams chats.
    pub presence_read: bool,
    /// Keep an Available application presence session active while the TUI runs.
    pub presence_primary: bool,
    /// Minutes of local inactivity before an automatic primary session becomes Away.
    /// Zero disables the automatic Away transition.
    pub presence_available_timeout_min: u64,
    /// Download regular Teams image file attachments from SharePoint/OneDrive.
    pub teams_file_images: bool,
    /// Experimental direct Entra directory profile lookup for Teams contacts.
    /// Off by default; requires delegated User.Read.All and administrator consent.
    pub directory_profile: bool,
    /// Optional persistent cache for decoded Teams image thumbnails.
    /// Unset keeps Teams image caching memory-only.
    pub teams_image_cache_dir: Option<PathBuf>,
    /// Maximum persistent Teams image cache size in MiB.
    pub teams_image_cache_max_mb: u64,
    /// Automatically prefill persistent Teams conversation caches in the background.
    /// Enabled by default; set M365_TEAMS_CACHE_WARMUP=0 to disable.
    pub teams_cache_warmup: bool,
    /// Which Teams system events are visible in conversations.
    /// Defaults to Useful; accepts useful, all, or none.
    pub teams_system_events: TeamsSystemEvents,
    /// Optional executable used to open Calendar online-meeting URLs.
    /// Unset uses the operating system handler (`xdg-open` / `open`).
    pub meeting_opener: Option<String>,
}

impl Config {
    /// Load configuration from the process environment. Call
    /// [`Config::load_dotenv`] first if you want `.env` support.
    pub fn from_env() -> Result<Self> {
        let client_id = env_required("M365_CLIENT_ID")?;
        let tenant_id = std::env::var("M365_TENANT_ID").unwrap_or_else(|_| "organizations".into());
        let presence_primary = env_flag("M365_PRESENCE_PRIMARY");
        let teams_file_images = env_flag("M365_TEAMS_FILE_IMAGES");
        let directory_profile = env_flag("M365_DIRECTORY_PROFILE");
        let teams_image_cache_dir = std::env::var("M365_TEAMS_IMAGE_CACHE_DIR")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let meeting_opener = std::env::var("M365_MEETING_OPENER")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let teams_cache_warmup = env_flag_default_on("M365_TEAMS_CACHE_WARMUP");
        let teams_system_events =
            parse_teams_system_events(std::env::var("M365_TEAMS_SYSTEM_EVENTS").ok().as_deref())?;
        let teams_image_cache_max_mb = match std::env::var("M365_TEAMS_IMAGE_CACHE_MAX_MB") {
            Ok(value) if !value.trim().is_empty() => {
                let value = value.trim().parse::<u64>().context(
                    "M365_TEAMS_IMAGE_CACHE_MAX_MB must be a positive integer number of MiB",
                )?;
                anyhow::ensure!(
                    value > 0,
                    "M365_TEAMS_IMAGE_CACHE_MAX_MB must be greater than zero"
                );
                value
            }
            _ => 256,
        };
        let presence_available_timeout_min =
            match std::env::var("M365_PRESENCE_AVAILABLE_TIMEOUT_MIN") {
                Ok(value) if !value.trim().is_empty() => value.trim().parse::<u64>().context(
                    "M365_PRESENCE_AVAILABLE_TIMEOUT_MIN must be an integer number of minutes",
                )?,
                _ => 5,
            };

        let scopes = match std::env::var("M365_SCOPES") {
            Ok(s) if !s.trim().is_empty() => s.split_whitespace().map(|s| s.to_string()).collect(),
            _ => {
                let mut s: Vec<String> = DEFAULT_SCOPES.iter().map(|s| s.to_string()).collect();
                // Presence *writing* is opt-in: adding a scope invalidates any
                // existing consent grant, which is disruptive in tenants that
                // require admin approval.
                if env_flag("M365_PRESENCE_WRITE") || presence_primary {
                    s.push(PRESENCE_WRITE_SCOPE.to_string());
                }
                if env_flag("M365_TEAMS_CHANNELS") {
                    s.push(TEAMS_READ_SCOPE.to_string());
                }
                if teams_file_images {
                    s.push(FILES_READ_SCOPE.to_string());
                }
                if env_flag("M365_PROFILE_PHOTO") {
                    s.push(PROFILE_PHOTO_SCOPE.to_string());
                }
                if directory_profile {
                    s.push(DIRECTORY_PROFILE_SCOPE.to_string());
                }
                s
            }
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

        // On unless explicitly disabled.
        let notifications = !matches!(
            std::env::var("M365_NOTIFY").as_deref(),
            Ok("0") | Ok("false") | Ok("no") | Ok("off")
        );
        let ntfy = parse_ntfy_mode(std::env::var("M365_NTFY").ok().as_deref())?;
        let ntfy_server = std::env::var("M365_NTFY_SERVER")
            .ok()
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty());
        let ntfy_topic = std::env::var("M365_NTFY_TOPIC")
            .ok()
            .map(|value| value.trim().trim_matches('/').to_string())
            .filter(|value| !value.is_empty());
        let ntfy_token = std::env::var("M365_NTFY_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        if ntfy.enabled() {
            let server = ntfy_server
                .as_deref()
                .context("M365_NTFY_SERVER is required when M365_NTFY is enabled")?;
            anyhow::ensure!(
                server.starts_with("http://") || server.starts_with("https://"),
                "M365_NTFY_SERVER must start with http:// or https://"
            );
            anyhow::ensure!(
                ntfy_topic.is_some(),
                "M365_NTFY_TOPIC is required when M365_NTFY is enabled"
            );
        }

        let calendar_notify =
            parse_calendar_notify(std::env::var("M365_CALENDAR_NOTIFY").ok().as_deref())?;
        let read_msg_timeout = match std::env::var("M365_READ_MSG_TIMEOUT") {
            Ok(value) if !value.trim().is_empty() => value
                .trim()
                .parse::<u64>()
                .context("M365_READ_MSG_TIMEOUT must be an integer number of seconds")?,
            _ => 0,
        };
        let presence_read = env_flag("M365_PRESENCE_READ");

        Ok(Self {
            client_id,
            tenant_id,
            scopes,
            tunnel_base_url,
            redis_url,
            token_cache_path,
            client_state,
            notifications,
            ntfy,
            ntfy_server,
            ntfy_topic,
            ntfy_token,
            calendar_notify,
            read_msg_timeout,
            presence_read,
            presence_primary,
            presence_available_timeout_min,
            teams_file_images,
            directory_profile,
            teams_image_cache_dir,
            teams_image_cache_max_mb,
            teams_cache_warmup,
            teams_system_events,
            meeting_opener,
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

    /// Whether the token we request can set presence.
    pub fn can_write_presence(&self) -> bool {
        self.has_scope(PRESENCE_WRITE_SCOPE)
    }

    /// Whether the token we request can enumerate teams and channels.
    pub fn can_read_teams(&self) -> bool {
        self.has_scope(TEAMS_READ_SCOPE)
    }

    /// Whether the current token request includes read access to files.
    pub fn can_read_files(&self) -> bool {
        self.has_scope(FILES_READ_SCOPE)
    }

    /// Whether the requested token can read other users' basic profile/photo.
    pub fn can_read_profile_photos(&self) -> bool {
        self.has_scope(PROFILE_PHOTO_SCOPE)
    }

    fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s.eq_ignore_ascii_case(scope))
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

fn parse_ntfy_mode(value: Option<&str>) -> Result<NtfyMode> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(NtfyMode::Never),
        Some(value) if value.eq_ignore_ascii_case("never") => Ok(NtfyMode::Never),
        Some(value) if value.eq_ignore_ascii_case("always") => Ok(NtfyMode::Always),
        Some(value) if value.eq_ignore_ascii_case("away") => Ok(NtfyMode::Away),
        Some(value) if value.eq_ignore_ascii_case("alwayswd") => Ok(NtfyMode::AlwaysWd),
        Some(value) if value.eq_ignore_ascii_case("awaywd") => Ok(NtfyMode::AwayWd),
        Some(value) => anyhow::bail!(
            "M365_NTFY must be one of: never, always, away, alwayswd, awaywd (got {value:?})"
        ),
    }
}

fn parse_calendar_notify(value: Option<&str>) -> Result<CalendarNotify> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(CalendarNotify::All),
        Some(value) if value.eq_ignore_ascii_case("all") => Ok(CalendarNotify::All),
        Some(value) if value.eq_ignore_ascii_case("internal") => Ok(CalendarNotify::Internal),
        Some(value) if value.eq_ignore_ascii_case("external") => Ok(CalendarNotify::External),
        Some(value) if value.eq_ignore_ascii_case("none") => Ok(CalendarNotify::None),
        Some(value) => anyhow::bail!(
            "M365_CALENDAR_NOTIFY must be one of: all, internal, external, none (got {value:?})"
        ),
    }
}

fn parse_teams_system_events(value: Option<&str>) -> Result<TeamsSystemEvents> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(TeamsSystemEvents::Useful),
        Some(value) if value.eq_ignore_ascii_case("useful") => Ok(TeamsSystemEvents::Useful),
        Some(value) if value.eq_ignore_ascii_case("all") => Ok(TeamsSystemEvents::All),
        Some(value) if value.eq_ignore_ascii_case("none") => Ok(TeamsSystemEvents::None),
        Some(value) => anyhow::bail!(
            "M365_TEAMS_SYSTEM_EVENTS must be one of: useful, all, none (got {value:?})"
        ),
    }
}

/// Enabled by default. Only `0`, `false`, `no`, or `off` disable it
/// (case-insensitive).
fn env_flag_default_on(key: &str) -> bool {
    std::env::var(key)
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

/// True for `1`, `true`, `yes`, `on` (case-insensitive).
fn env_flag(key: &str) -> bool {
    std::env::var(key)
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
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
    let dirs = directories::ProjectDirs::from("dev", "rootHytx", "m365-tui")
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
            notifications: true,
            ntfy: NtfyMode::Never,
            ntfy_server: None,
            ntfy_topic: None,
            ntfy_token: None,
            calendar_notify: CalendarNotify::All,
            read_msg_timeout: 0,
            presence_read: false,
            presence_primary: false,
            presence_available_timeout_min: 5,
            teams_file_images: false,
            directory_profile: false,
            teams_image_cache_dir: None,
            teams_image_cache_max_mb: 256,
            teams_cache_warmup: true,
            teams_system_events: TeamsSystemEvents::Useful,
            meeting_opener: None,
        }
    }

    #[test]
    fn parses_ntfy_mode() {
        assert_eq!(parse_ntfy_mode(None).unwrap(), NtfyMode::Never);
        assert_eq!(parse_ntfy_mode(Some(" ALWAYS ")).unwrap(), NtfyMode::Always);
        assert_eq!(parse_ntfy_mode(Some("away")).unwrap(), NtfyMode::Away);
        assert_eq!(
            parse_ntfy_mode(Some("alwayswd")).unwrap(),
            NtfyMode::AlwaysWd
        );
        assert_eq!(parse_ntfy_mode(Some("AWAYWD")).unwrap(), NtfyMode::AwayWd);
        assert_eq!(parse_ntfy_mode(Some("never")).unwrap(), NtfyMode::Never);
        assert!(parse_ntfy_mode(Some("sometimes")).is_err());

        assert!(!NtfyMode::Never.enabled());
        assert!(NtfyMode::Always.enabled());
        assert!(NtfyMode::Away.enabled());
        assert!(NtfyMode::AlwaysWd.enabled());
        assert!(NtfyMode::AwayWd.enabled());

        assert!(!NtfyMode::Always.needs_presence());
        assert!(NtfyMode::Away.needs_presence());
        assert!(!NtfyMode::AlwaysWd.needs_presence());
        assert!(NtfyMode::AwayWd.needs_presence());

        assert!(!NtfyMode::Always.needs_work_plan());
        assert!(!NtfyMode::Away.needs_work_plan());
        assert!(NtfyMode::AlwaysWd.needs_work_plan());
        assert!(NtfyMode::AwayWd.needs_work_plan());
    }

    #[test]
    fn parses_calendar_notification_mode() {
        assert_eq!(parse_calendar_notify(None).unwrap(), CalendarNotify::All);
        assert_eq!(
            parse_calendar_notify(Some(" internal ")).unwrap(),
            CalendarNotify::Internal
        );
        assert_eq!(
            parse_calendar_notify(Some("EXTERNAL")).unwrap(),
            CalendarNotify::External
        );
        assert_eq!(
            parse_calendar_notify(Some("none")).unwrap(),
            CalendarNotify::None
        );
        assert!(parse_calendar_notify(Some("desktop")).is_err());

        assert!(CalendarNotify::All.internal());
        assert!(CalendarNotify::All.external());
        assert!(CalendarNotify::Internal.internal());
        assert!(!CalendarNotify::Internal.external());
        assert!(!CalendarNotify::External.internal());
        assert!(CalendarNotify::External.external());
        assert!(!CalendarNotify::None.enabled());
    }

    #[test]
    fn parses_teams_system_event_visibility() {
        assert_eq!(
            parse_teams_system_events(None).unwrap(),
            TeamsSystemEvents::Useful
        );
        assert_eq!(
            parse_teams_system_events(Some(" useful ")).unwrap(),
            TeamsSystemEvents::Useful
        );
        assert_eq!(
            parse_teams_system_events(Some("ALL")).unwrap(),
            TeamsSystemEvents::All
        );
        assert_eq!(
            parse_teams_system_events(Some("none")).unwrap(),
            TeamsSystemEvents::None
        );
        assert!(parse_teams_system_events(Some("usefull")).is_err());
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
    fn graph_base_defaults_to_the_real_endpoint() {
        // Serialised with the override test below: both touch process env.
        std::env::remove_var("M365_GRAPH_BASE");
        assert_eq!(graph_base(), GRAPH_BASE);

        std::env::set_var("M365_GRAPH_BASE", "http://127.0.0.1:8765/v1.0/");
        assert_eq!(graph_base(), "http://127.0.0.1:8765/v1.0");

        // Empty means "not set", so an unfilled .env entry can't break the app.
        std::env::set_var("M365_GRAPH_BASE", "  ");
        assert_eq!(graph_base(), GRAPH_BASE);
        std::env::remove_var("M365_GRAPH_BASE");
    }

    #[test]
    fn no_tunnel_means_no_notification_url() {
        let c = sample(None);
        assert!(c.notification_url().is_none());
    }
}
