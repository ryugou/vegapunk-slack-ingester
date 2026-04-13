use anyhow::Result;
use chrono::{TimeZone, Utc};
use tracing::info;

use crate::slack::client::SlackClient;
use crate::slack::types::HistoryMessage;
use crate::vegapunk::VegapunkClient;
use crate::SCHEMA_NAME;

/// A Slack message with resolved user and channel names.
#[derive(Debug, Clone)]
pub struct SlackMessage {
    pub text: String,
    pub user_id: String,
    pub user_name: String,
    pub channel_id: String,
    pub channel_name: String,
    pub ts: String,
    pub thread_ts: Option<String>,
}

/// A message in the format expected by the Vegapunk ingest API.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IngestMessage {
    pub id: String,
    pub text: String,
    pub metadata: IngestMetadata,
}

/// Metadata attached to an ingested message.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IngestMetadata {
    pub source_type: String,
    pub author: String,
    pub author_id: String,
    pub channel: String,
    pub channel_id: String,
    pub thread_id: Option<String>,
    pub timestamp: String,
}

/// Convert a Slack message to the Vegapunk ingest format.
///
/// `thread_id` mapping: Slack replies have `thread_ts` set to the parent's
/// `ts`. Parent messages (thread starters) have `thread_ts = None`, but
/// they implicitly belong to a thread identified by their own `ts`. We
/// normalize this by falling back to `msg.ts` when `thread_ts` is absent,
/// ensuring parent and replies share the same Thread node in vegapunk.
pub fn slack_to_ingest(msg: &SlackMessage) -> IngestMessage {
    let id = format!("{}-{}", msg.channel_id, msg.ts);
    let timestamp = slack_ts_to_rfc3339(&msg.ts);

    // Parent messages: thread_ts is None → use own ts as thread_id
    // Reply messages: thread_ts is Some(parent_ts) → use as-is
    let thread_id = Some(
        msg.thread_ts
            .clone()
            .unwrap_or_else(|| msg.ts.clone()),
    );

    IngestMessage {
        id,
        text: msg.text.clone(),
        metadata: IngestMetadata {
            source_type: "slack".to_string(),
            author: msg.user_name.clone(),
            author_id: msg.user_id.clone(),
            channel: msg.channel_name.clone(),
            channel_id: msg.channel_id.clone(),
            thread_id,
            timestamp,
        },
    }
}

/// Convert a batch of Slack API history messages into `SlackMessage` structs,
/// resolving user names via the Slack client cache. Thread replies are fetched
/// and appended after their parent message.
///
/// `user_token` is used to authenticate file downloads for attachment extraction.
pub async fn history_to_slack_messages(
    msgs: &[HistoryMessage],
    channel_id: &str,
    channel_name: &str,
    slack: &mut SlackClient,
    user_token: &str,
) -> Result<Vec<SlackMessage>> {
    let mut batch: Vec<SlackMessage> = Vec::new();

    for msg in msgs {
        if msg.bot_id.is_some() {
            continue;
        }
        let Some(ref user_id) = msg.user else {
            continue;
        };
        let text = msg.text.as_deref().unwrap_or("");
        let has_files = msg.files.as_ref().is_some_and(|f| !f.is_empty());

        // Skip messages with no text AND no files
        if text.is_empty() && !has_files {
            continue;
        }

        let user_name = slack
            .get_user_name(user_id)
            .await
            .unwrap_or_else(|_| user_id.clone());

        let enriched = crate::extractor::enrich_text(text, msg.files.as_deref(), user_token).await;

        // After enrichment, skip if still empty
        if enriched.trim().is_empty() {
            continue;
        }

        batch.push(SlackMessage {
            text: enriched,
            user_id: user_id.clone(),
            user_name,
            channel_id: channel_id.to_string(),
            channel_name: channel_name.to_string(),
            ts: msg.ts.clone(),
            thread_ts: msg.thread_ts.clone(),
        });

        if msg.reply_count.unwrap_or(0) > 0 && msg.thread_ts.is_none() {
            let replies = slack.conversations_replies(channel_id, &msg.ts).await?;
            for reply in &replies.messages {
                if reply.ts == msg.ts {
                    continue;
                }
                if reply.bot_id.is_some() {
                    continue;
                }
                let Some(ref reply_user) = reply.user else {
                    continue;
                };
                let reply_text = reply.text.as_deref().unwrap_or("");
                let reply_has_files = reply.files.as_ref().is_some_and(|f| !f.is_empty());

                if reply_text.is_empty() && !reply_has_files {
                    continue;
                }

                let reply_user_name = slack
                    .get_user_name(reply_user)
                    .await
                    .unwrap_or_else(|_| reply_user.clone());

                let enriched_reply =
                    crate::extractor::enrich_text(reply_text, reply.files.as_deref(), user_token)
                        .await;

                if enriched_reply.trim().is_empty() {
                    continue;
                }

                batch.push(SlackMessage {
                    text: enriched_reply,
                    user_id: reply_user.clone(),
                    user_name: reply_user_name,
                    channel_id: channel_id.to_string(),
                    channel_name: channel_name.to_string(),
                    ts: reply.ts.clone(),
                    thread_ts: reply.thread_ts.clone(),
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        }
    }

    Ok(batch)
}

/// Sort messages by ts and ingest them in batches via the Vegapunk client.
pub async fn ingest_batch(
    batch: &mut [SlackMessage],
    vegapunk: &mut VegapunkClient,
    batch_size: usize,
    channel_id: &str,
) -> Result<u32> {
    batch.sort_by(|a, b| a.ts.cmp(&b.ts));
    let mut total: u32 = 0;

    for chunk in batch.chunks(batch_size) {
        let ingest_msgs: Vec<_> = chunk.iter().map(slack_to_ingest).collect();
        let count = vegapunk.ingest(ingest_msgs, SCHEMA_NAME).await?;
        total += count as u32;
        info!(channel_id, ingested = count, "batch ingested");
    }

    Ok(total)
}

/// Convert an RFC3339 timestamp back to Slack ts format (e.g. "1712345678.000000").
///
/// Used when recovering cursors from Vegapunk (which stores RFC3339) and passing
/// to Slack API (which expects Unix epoch ts).
pub fn rfc3339_to_slack_ts(rfc3339: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .map(|dt| format!("{}.000000", dt.timestamp()))
}

/// Convert a Slack timestamp (e.g. "1712345678.123456") to RFC3339 format.
fn slack_ts_to_rfc3339(ts: &str) -> String {
    let secs: i64 = ts
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    // SAFETY: timestamp_opt returns None only for out-of-range values;
    // Slack timestamps are always valid Unix epoch values.
    let dt = Utc.timestamp_opt(secs, 0).single().unwrap_or_else(Utc::now);
    dt.to_rfc3339()
}
