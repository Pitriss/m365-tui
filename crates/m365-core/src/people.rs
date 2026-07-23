//! People lookup and presence — used for cross-navigation (email <-> chat) and
//! status dots.

use anyhow::Result;
use serde_json::json;

use crate::graph::GraphClient;
use crate::models::{Person, Presence, User};

pub async fn me(graph: &GraphClient) -> Result<User> {
    graph
        .get_json("me?$select=id,displayName,mail,userPrincipalName,jobTitle")
        .await
}

/// Relevant people for the signed-in user, optionally filtered by a search term
/// (matches name or email).
pub async fn relevant_people(graph: &GraphClient, search: Option<&str>) -> Result<Vec<Person>> {
    let path = match search {
        Some(q) => format!("me/people?$search=\"{}\"&$top=25", q.replace('"', "")),
        None => "me/people?$top=25".to_string(),
    };
    graph.get_collection(&path).await
}

/// Resolve a user id from an email address (used to open a chat with an email
/// sender). Returns `None` if the address is not a known directory user.
pub async fn user_id_for_email(graph: &GraphClient, email: &str) -> Result<Option<String>> {
    // /users/{email} accepts the UPN/mail directly for directory members.
    match graph
        .get_json::<User>(&format!("users/{email}?$select=id"))
        .await
    {
        Ok(u) => Ok(Some(u.id)),
        Err(_) => Ok(None),
    }
}

/// Presence for a set of user ids (Teams status dots).
pub async fn presences(graph: &GraphClient, user_ids: &[String]) -> Result<Vec<Presence>> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let payload = json!({ "ids": user_ids });
    #[derive(serde::Deserialize)]
    struct Wrapper {
        value: Vec<Presence>,
    }
    let w: Wrapper = graph
        .post_json("communications/getPresencesByUserId", &payload)
        .await?;
    Ok(w.value)
}
