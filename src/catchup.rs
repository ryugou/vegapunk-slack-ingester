use anyhow::Result;
use tracing::{info, warn};

use crate::config::Config;
use crate::converter::{history_to_slack_messages, ingest_batch, rfc3339_to_slack_ts};
use crate::cursor::CursorStore;
use crate::slack::SlackClient;
use crate::vegapunk::VegapunkClient;
use crate::SCHEMA_NAME;

/// Catch up on messages sent while the ingester was offline.
///
/// For each watched channel, reads the local cursor (or falls back to querying
/// Vegapunk) and ingests all messages newer than the cursor.
pub async fn run_catchup(
    config: &Config,
    slack: &mut SlackClient,
    vegapunk: &mut VegapunkClient,
    cursor_store: &CursorStore,
) -> Result<()> {
    let cursors = cursor_store.load()?;
    let channels = &config.slack_watch_channel_ids;

    if channels.is_empty() {
        info!("no watch channels configured, skipping catchup");
        return Ok(());
    }

    for channel_id in channels {
        let oldest = if let Some(ts) = cursors.get(channel_id) {
            info!(channel_id, cursor = %ts, "catching up from local cursor");
            ts.clone()
        } else {
            match vegapunk
                .get_latest_message_ts(SCHEMA_NAME, channel_id)
                .await
            {
                Ok(Some(rfc3339_ts)) => {
                    // Vegapunk stores RFC3339; Slack API expects Unix epoch ts
                    let slack_ts = rfc3339_to_slack_ts(&rfc3339_ts).unwrap_or(rfc3339_ts);
                    info!(channel_id, cursor = %slack_ts, "catching up from Vegapunk QueryNodes");
                    slack_ts
                }
                Ok(None) => {
                    info!(channel_id, "no data in Vegapunk, skipping catchup");
                    continue;
                }
                Err(e) => {
                    warn!(channel_id, error = %e, "failed to query Vegapunk for cursor, skipping");
                    continue;
                }
            }
        };

        let channel_name = slack
            .get_channel_name(channel_id)
            .await
            .unwrap_or_else(|_| channel_id.clone());

        let mut cursor_page: Option<String> = None;
        let mut last_ts: Option<String> = None;

        loop {
            let history = slack
                .conversations_history(channel_id, Some(&oldest), cursor_page.as_deref(), 200)
                .await?;

            let mut batch = history_to_slack_messages(
                &history.messages,
                channel_id,
                &channel_name,
                slack,
                &config.slack_user_token,
            )
            .await?;

            ingest_batch(&mut batch, vegapunk, config.ingest_batch_size, channel_id).await?;

            // Save the maximum ts after sorting (ingest_batch sorts the batch)
            if let Some(max_ts) = batch.iter().map(|m| &m.ts).max() {
                last_ts = Some(max_ts.clone());
            }

            if let Some(ref ts) = last_ts {
                cursor_store.save_channel(channel_id, ts)?;
            }

            let has_more = history.has_more.unwrap_or(false);
            if !has_more {
                break;
            }
            cursor_page = history
                .response_metadata
                .and_then(|m| m.next_cursor)
                .filter(|c| !c.is_empty());
            if cursor_page.is_none() {
                break;
            }

            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        }

        info!(channel_id, "catchup complete");
    }

    Ok(())
}
