//! Outlook mail endpoints.

use anyhow::Result;
use serde_json::json;

use crate::graph::{DeltaPage, GraphClient};
use crate::models::{MailFolder, MailMessage};

/// List mail folders (Inbox, Sent Items, custom folders, ...).
pub async fn list_folders(graph: &GraphClient) -> Result<Vec<MailFolder>> {
    graph
        .get_collection("me/mailFolders?$top=100&$select=id,displayName,unreadItemCount,totalItemCount")
        .await
}

/// List the first page of messages in a folder, newest first. Returns the page
/// and the `@odata.nextLink` for "load more" (if the folder has more).
pub async fn list_messages(
    graph: &GraphClient,
    folder_id: &str,
    top: u32,
) -> Result<(Vec<MailMessage>, Option<String>)> {
    let path = format!(
        "me/mailFolders/{folder_id}/messages?$top={top}&$orderby=receivedDateTime desc\
         &$select=id,subject,bodyPreview,from,toRecipients,receivedDateTime,isRead,hasAttachments,webLink"
    );
    // Single page only — `$top` bounds it; we don't want to walk the whole folder.
    graph.get_page_with_next(&path).await
}

/// Fetch the next page of messages from an `@odata.nextLink` returned by
/// [`list_messages`].
pub async fn list_messages_more(
    graph: &GraphClient,
    next_link: &str,
) -> Result<(Vec<MailMessage>, Option<String>)> {
    graph.get_page_with_next(next_link).await
}

/// Fetch a single message including its full body.
pub async fn get_message(graph: &GraphClient, id: &str) -> Result<MailMessage> {
    let path = format!(
        "me/messages/{id}?$select=id,subject,body,bodyPreview,from,toRecipients,receivedDateTime,isRead,hasAttachments,webLink"
    );
    graph.get_json(&path).await
}

/// Incremental sync of a folder. Pass `None` for the first call, then feed back
/// the returned `delta_link` on each subsequent poll.
pub async fn delta_messages(
    graph: &GraphClient,
    folder_id: &str,
    delta_link: Option<&str>,
) -> Result<DeltaPage<MailMessage>> {
    let path = match delta_link {
        Some(link) => link.to_string(),
        None => format!("me/mailFolders/{folder_id}/messages/delta?$select=id,subject,bodyPreview,from,receivedDateTime,isRead"),
    };
    graph.delta(&path).await
}

/// Search across the mailbox using Graph `$search`.
pub async fn search(graph: &GraphClient, query: &str, top: u32) -> Result<Vec<MailMessage>> {
    // $search requires ConsistencyLevel semantics; Graph accepts the quoted form.
    let escaped = query.replace('"', "");
    let path = format!(
        "me/messages?$search=\"{escaped}\"&$top={top}\
         &$select=id,subject,bodyPreview,from,receivedDateTime,isRead"
    );
    graph.get_page(&path).await
}

pub async fn mark_read(graph: &GraphClient, id: &str, read: bool) -> Result<()> {
    graph
        .patch(&format!("me/messages/{id}"), &json!({ "isRead": read }))
        .await
}

/// Send a new message.
pub async fn send_mail(graph: &GraphClient, to: &[String], subject: &str, body: &str) -> Result<()> {
    let recipients: Vec<_> = to
        .iter()
        .map(|addr| json!({ "emailAddress": { "address": addr } }))
        .collect();
    let payload = json!({
        "message": {
            "subject": subject,
            "body": { "contentType": "Text", "content": body },
            "toRecipients": recipients,
        },
        "saveToSentItems": true,
    });
    graph.post_action("me/sendMail", &payload).await
}

/// Reply to a message (Graph fills quoting + recipients automatically).
pub async fn reply(graph: &GraphClient, id: &str, comment: &str) -> Result<()> {
    graph
        .post_action(&format!("me/messages/{id}/reply"), &json!({ "comment": comment }))
        .await
}

/// Reply-all to a message.
pub async fn reply_all(graph: &GraphClient, id: &str, comment: &str) -> Result<()> {
    graph
        .post_action(&format!("me/messages/{id}/replyAll"), &json!({ "comment": comment }))
        .await
}
