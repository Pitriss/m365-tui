//! Serde models for the subset of Microsoft Graph resources the TUI uses.
//! Fields are intentionally partial — Graph returns far more than we render.

use serde::{Deserialize, Serialize};

/// The signed-in user (`/me`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub mail: Option<String>,
    #[serde(default)]
    pub user_principal_name: Option<String>,
    #[serde(default)]
    pub job_title: Option<String>,
}

impl User {
    pub fn best_email(&self) -> Option<&str> {
        self.mail
            .as_deref()
            .or(self.user_principal_name.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailAddress {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipient {
    #[serde(default)]
    pub email_address: Option<EmailAddress>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemBody {
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}

// ---------------------------------------------------------------------------
// Mail
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailFolder {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub unread_item_count: Option<i64>,
    #[serde(default)]
    pub total_item_count: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailMessage {
    pub id: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub body_preview: Option<String>,
    #[serde(default)]
    pub from: Option<Recipient>,
    #[serde(default)]
    pub to_recipients: Vec<Recipient>,
    #[serde(default)]
    pub received_date_time: Option<String>,
    #[serde(default)]
    pub is_read: Option<bool>,
    #[serde(default)]
    pub has_attachments: Option<bool>,
    #[serde(default)]
    pub web_link: Option<String>,
    /// Populated only when a single message is fetched with `$select=body`.
    #[serde(default)]
    pub body: Option<ItemBody>,
}

impl MailMessage {
    pub fn sender_name(&self) -> String {
        self.from
            .as_ref()
            .and_then(|r| r.email_address.as_ref())
            .and_then(|e| e.name.clone().or_else(|| e.address.clone()))
            .unwrap_or_else(|| "(unknown)".into())
    }

    pub fn sender_address(&self) -> Option<String> {
        self.from
            .as_ref()
            .and_then(|r| r.email_address.as_ref())
            .and_then(|e| e.address.clone())
    }
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DateTimeTimeZone {
    pub date_time: String,
    #[serde(default)]
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attendee {
    #[serde(default)]
    pub email_address: Option<EmailAddress>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub start: Option<DateTimeTimeZone>,
    #[serde(default)]
    pub end: Option<DateTimeTimeZone>,
    #[serde(default)]
    pub organizer: Option<Recipient>,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
    #[serde(default)]
    pub location: Option<Location>,
    #[serde(default)]
    pub is_online_meeting: Option<bool>,
    #[serde(default)]
    pub online_meeting: Option<OnlineMeetingInfo>,
    #[serde(default)]
    pub body_preview: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineMeetingInfo {
    #[serde(default)]
    pub join_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Teams: chats, channels, messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub id: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub chat_type: Option<String>,
    #[serde(default)]
    pub last_updated_date_time: Option<String>,
    #[serde(default)]
    pub members: Vec<ConversationMember>,
    #[serde(default)]
    pub last_message_preview: Option<LastMessagePreview>,
}

impl Chat {
    /// A display label: the explicit topic, else the member names joined.
    pub fn label(&self, me_id: Option<&str>) -> String {
        if let Some(t) = self.topic.as_ref().filter(|t| !t.is_empty()) {
            return t.clone();
        }
        let names: Vec<String> = self
            .members
            .iter()
            .filter(|m| me_id.map(|id| m.user_id.as_deref() != Some(id)).unwrap_or(true))
            .filter_map(|m| m.display_name.clone())
            .collect();
        if names.is_empty() {
            self.chat_type.clone().unwrap_or_else(|| "chat".into())
        } else {
            names.join(", ")
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMember {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastMessagePreview {
    #[serde(default)]
    pub body: Option<ItemBody>,
    #[serde(default)]
    pub created_date_time: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySet {
    #[serde(default)]
    pub user: Option<Identity>,
    #[serde(default)]
    pub application: Option<Identity>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

/// A Teams `chatMessage` (channel or chat).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    #[serde(default)]
    pub created_date_time: Option<String>,
    #[serde(default)]
    pub from: Option<IdentitySet>,
    #[serde(default)]
    pub body: Option<ItemBody>,
    #[serde(default)]
    pub message_type: Option<String>,
    #[serde(default)]
    pub deleted_date_time: Option<String>,
    #[serde(default)]
    pub reactions: Vec<MessageReaction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageReaction {
    #[serde(default)]
    pub reaction_type: Option<String>,
}

impl ChatMessage {
    pub fn author(&self) -> String {
        self.from
            .as_ref()
            .and_then(|f| f.user.as_ref().or(f.application.as_ref()))
            .and_then(|i| i.display_name.clone())
            .unwrap_or_else(|| "(system)".into())
    }

    pub fn text(&self) -> String {
        self.body
            .as_ref()
            .and_then(|b| b.content.clone())
            .unwrap_or_default()
    }

    /// A short display of reactions, e.g. `👍 ❤️`. Maps the classic Teams
    /// reaction names to emoji; unicode reactions pass through as-is.
    pub fn reactions_summary(&self) -> Option<String> {
        if self.reactions.is_empty() {
            return None;
        }
        let mut out = String::new();
        for r in &self.reactions {
            let ty = r.reaction_type.as_deref().unwrap_or("");
            let e = match ty {
                "like" => "👍",
                "heart" => "❤️",
                "laugh" => "😆",
                "surprised" => "😮",
                "sad" => "😢",
                "angry" => "😠",
                other => other,
            };
            if !e.is_empty() {
                out.push_str(e);
                out.push(' ');
            }
        }
        let out = out.trim_end().to_string();
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

// ---------------------------------------------------------------------------
// People & presence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default, rename = "scoredEmailAddresses")]
    pub scored_email_addresses: Vec<ScoredEmailAddress>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoredEmailAddress {
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub availability: Option<String>,
    #[serde(default)]
    pub activity: Option<String>,
}

/// Outbound draft used by the compose views for both mail and Teams messages.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Draft {
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
}
