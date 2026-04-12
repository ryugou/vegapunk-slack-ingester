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
pub fn slack_to_ingest(msg: &SlackMessage) -> IngestMessage {
    let id = format!("{}-{}", msg.channel_id, msg.ts);
    let timestamp = slack_ts_to_rfc3339(&msg.ts);

    IngestMessage {
        id,
        text: msg.text.clone(),
        metadata: IngestMetadata {
            source_type: "slack".to_string(),
            author: msg.user_name.clone(),
            author_id: msg.user_id.clone(),
            channel: msg.channel_name.clone(),
            channel_id: msg.channel_id.clone(),
            thread_id: msg.thread_ts.clone(),
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
        let Some(ref text) = msg.text else {
            continue;
        };
        if text.is_empty() {
            continue;
        }

        let user_name = slack
            .get_user_name(user_id)
            .await
            .unwrap_or_else(|_| user_id.clone());

        let file_text = if let Some(ref files) = msg.files {
            crate::extractor::extract_files(files, user_token).await
        } else {
            String::new()
        };
        let link_text = crate::extractor::link_titles(text).await;
        let enriched = format!("{text}{file_text}{link_text}");

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
                let Some(ref reply_text) = reply.text else {
                    continue;
                };
                if reply_text.is_empty() {
                    continue;
                }
                let reply_user_name = slack
                    .get_user_name(reply_user)
                    .await
                    .unwrap_or_else(|_| reply_user.clone());

                let reply_file_text = if let Some(ref files) = reply.files {
                    crate::extractor::extract_files(files, user_token).await
                } else {
                    String::new()
                };
                let reply_link_text = crate::extractor::link_titles(reply_text).await;
                let enriched_reply = format!("{reply_text}{reply_file_text}{reply_link_text}");

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
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
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
