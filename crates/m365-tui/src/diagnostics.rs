//! Read-only F6 diagnostics and safe export helpers.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::{DateTime, Local, Utc};
use m365_core::auth::TokenInfo;
use m365_core::config::NtfyMode;
use m365_core::work_plan::WorkPlanDiagnostics;

use crate::app::{App, PushState};

#[derive(Debug, Clone)]
pub struct DiagnosticsRemote {
    pub generated_at: DateTime<Utc>,
    pub token: Result<TokenInfo, String>,
    pub work_plan: Result<WorkPlanDiagnostics, String>,
}

#[derive(Debug, Default)]
pub struct DiagnosticsState {
    pub loading: bool,
    pub remote: Option<DiagnosticsRemote>,
}

fn row(label: &str, value: impl AsRef<str>) -> String {
    format!("  {label:<30} {}", value.as_ref())
}

fn local_timezone_name() -> String {
    if let Ok(value) = std::env::var("TZ") {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }

    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        let value = target.to_string_lossy();
        if let Some((_, zone)) = value.split_once("/zoneinfo/") {
            if !zone.trim().is_empty() {
                return zone.to_string();
            }
        }
    }

    chrono::Local::now().offset().to_string()
}

fn ntfy_mode(mode: NtfyMode) -> &'static str {
    match mode {
        NtfyMode::Never => "never",
        NtfyMode::Always => "always",
        NtfyMode::Away => "away",
        NtfyMode::AlwaysWd => "alwayswd",
        NtfyMode::AwayWd => "awaywd",
    }
}

fn yes_no(value: bool, yes: &str, no: &str) -> String {
    if value {
        format!("● {yes}")
    } else {
        format!("○ {no}")
    }
}

fn compact_error(value: &str) -> String {
    let flat = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = flat.chars();
    let short: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

fn permission_lines(app: &App, token: Option<&TokenInfo>) -> Vec<String> {
    const SCOPES: &[&str] = &[
        "User.Read",
        "People.Read",
        "Mail.ReadWrite",
        "Mail.Send",
        "Calendars.ReadWrite",
        "Chat.ReadWrite",
        "ChannelMessage.Send",
        "ChannelMessage.Read.All",
        "Presence.Read.All",
        "Presence.ReadWrite",
        "Team.ReadBasic.All",
        "Files.Read.All",
        "User.ReadBasic.All",
        "User.Read.All",
    ];

    let requested: HashSet<String> = app
        .session
        .config
        .scopes
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect();
    let granted: Option<HashSet<String>> = token.map(|token| {
        token
            .scopes
            .iter()
            .map(|value| value.to_ascii_lowercase())
            .collect()
    });

    SCOPES
        .iter()
        .map(|scope| {
            let key = scope.to_ascii_lowercase();
            let requested = requested.contains(&key);
            let granted = granted.as_ref().map(|items| items.contains(&key));
            let state = match (requested, granted) {
                (true, Some(true)) => "● granted".to_string(),
                (true, Some(false)) => "! requested, missing from token".to_string(),
                (false, Some(true)) => "! granted, not requested now".to_string(),
                (false, Some(false)) => "○ not requested".to_string(),
                (true, None) => "? requested, token scopes unavailable".to_string(),
                (false, None) => "○ not requested".to_string(),
            };
            row(scope, state)
        })
        .collect()
}

pub fn text(app: &App) -> String {
    let mut out = Vec::new();
    out.push("m365-tui diagnostics".to_string());

    let remote = app.diagnostics.remote.as_ref();
    if let Some(remote) = remote {
        out.push(format!(
            "Generated: {}",
            remote
                .generated_at
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S %:z")
        ));
    } else if app.diagnostics.loading {
        out.push("Generated: loading…".into());
    }
    out.push(format!("Version: {}", env!("CARGO_PKG_VERSION")));
    out.push(String::new());

    out.push("Identity / Token".into());
    let account = app
        .me
        .as_ref()
        .and_then(|user| user.best_email())
        .unwrap_or("unknown");
    out.push(row("Account", account));
    if let Some(name) = app
        .me
        .as_ref()
        .and_then(|user| user.display_name.as_deref())
        .filter(|value| !value.trim().is_empty())
    {
        out.push(row("Display name", name));
    }
    out.push(row("Tenant", &app.session.config.tenant_id));
    out.push(row("Client", &app.session.config.client_id));

    let token = remote.and_then(|remote| remote.token.as_ref().ok());
    match remote.map(|remote| &remote.token) {
        Some(Ok(token)) => {
            out.push(row(
                "Token",
                if token.valid { "● valid" } else { "! expired" },
            ));
            out.push(row(
                "Token expires",
                token
                    .expires_at
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S %:z")
                    .to_string(),
            ));
        }
        Some(Err(error)) => out.push(row(
            "Token",
            format!("× {}", compact_error(error)),
        )),
        None => out.push(row(
            "Token",
            if app.diagnostics.loading {
                "? loading"
            } else {
                "? not loaded"
            },
        )),
    }
    out.push(String::new());

    out.push("Microsoft 365 work settings".into());
    out.push(row("Local time zone", local_timezone_name()));
    out.push(row("NTFY mode", ntfy_mode(app.session.config.ntfy)));
    match remote.map(|remote| &remote.work_plan) {
        Some(Ok(plan)) => {
            out.push(row(
                "Work plan now",
                if plan.working_now {
                    "● working"
                } else {
                    "○ outside working plan"
                },
            ));
            if plan.recurrences.is_empty() {
                out.push(row("Recurring schedule", "○ none returned"));
            } else {
                for (index, recurrence) in plan.recurrences.iter().enumerate() {
                    let days = if recurrence.days.is_empty() {
                        "?".to_string()
                    } else {
                        recurrence.days.join(" ")
                    };
                    let zone = recurrence.time_zone.as_deref().unwrap_or("unknown zone");
                    let location = recurrence.location.as_deref().unwrap_or("unspecified");
                    out.push(row(
                        &format!("Schedule {}", index + 1),
                        format!(
                            "{days} · {}-{} · {zone} · {location}",
                            recurrence.start_time, recurrence.end_time
                        ),
                    ));
                }
            }
        }
        Some(Err(error)) => out.push(row(
            "Work plan",
            format!("× {}", compact_error(error)),
        )),
        None => out.push(row(
            "Work plan",
            if app.diagnostics.loading {
                "? loading"
            } else {
                "? not loaded"
            },
        )),
    }
    out.push(String::new());

    out.push("Microsoft Graph permissions".into());
    out.extend(permission_lines(app, token));
    out.push(String::new());

    out.push("Optional features".into());
    out.push(row(
        "Contact presence",
        yes_no(app.session.config.presence_read, "enabled", "disabled"),
    ));
    out.push(row(
        "Primary presence",
        yes_no(
            app.session.config.presence_primary,
            "enabled",
            "disabled",
        ),
    ));
    out.push(row(
        "Teams channels",
        yes_no(
            app.session.config.can_read_teams(),
            "enabled",
            "disabled",
        ),
    ));
    out.push(row(
        "Teams file images",
        yes_no(
            app.session.config.teams_file_images,
            "enabled",
            "disabled",
        ),
    ));
    out.push(row(
        "Profile photos",
        yes_no(
            app.session.config.can_read_profile_photos(),
            "enabled",
            "disabled",
        ),
    ));
    out.push(row(
        "Directory profile",
        yes_no(
            app.session.config.directory_profile,
            "enabled",
            "disabled",
        ),
    ));
    out.push(String::new());

    out.push("Teams / Presence".into());
    match app.my_presence.as_ref() {
        Some(presence) => {
            let availability = presence.availability.as_deref().unwrap_or("unknown");
            let activity = presence.activity.as_deref().unwrap_or("unknown");
            out.push(row(
                "Graph presence",
                format!("● {availability} · {activity}"),
            ));
        }
        None => out.push(row("Graph presence", "? not loaded")),
    }
    out.push(row("Skype Presence Service", "? not verified"));
    out.push(row("Skype Presence R/W", "? no Skype resource token"));
    out.push(String::new());

    out.push("Runtime".into());
    let push = match &app.push {
        PushState::Off => "○ poll only".to_string(),
        PushState::Connecting => "? connecting".to_string(),
        PushState::Live => "● live".to_string(),
        PushState::Failed(error) => format!("× failed: {}", compact_error(error)),
    };
    out.push(row("Push", push));
    out.push(row(
        "Persistent Teams cache",
        yes_no(
            app.session.config.teams_image_cache_dir.is_some(),
            "enabled",
            "disabled",
        ),
    ));
    out.push(row(
        "Kitty image protocol",
        yes_no(
            app.kitty_images_available(),
            "available",
            "unavailable",
        ),
    ));
    out.push(row(
        "Clipboard helper",
        crate::clipboard::native_backend()
            .map(|value| format!("● {value}"))
            .unwrap_or_else(|| "○ none; diagnostics use log fallback".into()),
    ));
    out.push(row(
        "Last Graph poll",
        format!("{}s ago", app.poll_elapsed().as_secs()),
    ));
    if let Some(kb) = app.rss_kb {
        out.push(row("RSS", format!("{:.1} MiB", kb as f64 / 1024.0)));
    }

    out.join("\n")
}

/// Write diagnostics to a private timestamped file in the platform temp dir.
pub fn save_log(text: &str) -> anyhow::Result<PathBuf> {
    use anyhow::Context;

    let name = format!(
        "m365-tui-diagnostics-{}.log",
        Local::now().format("%Y%m%d-%H%M%S")
    );
    let path = std::env::temp_dir().join(name);

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("creating {}", path.display()))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        file.flush()
            .with_context(|| format!("flushing {}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        std::fs::write(&path, text)
            .with_context(|| format!("writing {}", path.display()))?;
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::compact_error;

    #[test]
    fn compact_error_is_single_line_and_bounded() {
        let value = format!("first\nsecond {}", "x".repeat(300));
        let result = compact_error(&value);
        assert!(!result.contains('\n'));
        assert!(result.chars().count() <= 121);
    }
}
