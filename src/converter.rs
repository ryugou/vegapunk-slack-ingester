use chrono::{TimeZone, Utc};

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
    #[serde(skip)]
    pub schema: String,
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
pub fn slack_to_ingest(msg: &SlackMessage, schema: &str) -> IngestMessage {
    let id = format!("{}-{}", msg.channel_id, msg.ts);
    let timestamp = slack_ts_to_rfc3339(&msg.ts);

    IngestMessage {
        id,
        text: msg.text.clone(),
        schema: schema.to_string(),
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
