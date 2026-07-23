//! Cross-navigation helpers that link the Outlook and Teams sides together.

use anyhow::Result;
use m365_core::{chats, people, Session};

/// Resolve an email address to a directory user and open (creating if needed) a
/// 1:1 Teams chat with them. Returns the chat id, or `None` if the address is
/// not a directory user we can chat with.
pub async fn chat_id_for_email(
    session: &Session,
    my_user_id: &str,
    email: &str,
) -> Result<Option<String>> {
    let Some(other) = people::user_id_for_email(&session.graph, email).await? else {
        return Ok(None);
    };
    let chat = chats::create_one_on_one(&session.graph, my_user_id, &other).await?;
    Ok(Some(chat.id))
}
