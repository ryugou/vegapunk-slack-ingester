use anyhow::Result;
use tracing::{info, warn};

use crate::config::Config;
use crate::converter::{slack_to_ingest, SlackMessage};
use crate::cursor::CursorStore;
use crate::slack::SlackClient;
use crate::vegapunk::VegapunkClient;

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
                .get_latest_message_ts("slack-ingester", channel_id)
                .await
            {
                Ok(Some(ts)) => {
                    info!(channel_id, cursor = %ts, "catching up from Vegapunk QueryNodes");
                    ts
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

            let mut batch: Vec<SlackMessage> = Vec::new();

            for msg in &history.messages {
                if msg.bot_id.is_some() {
                    continue;
                }
                let Some(ref user_id) = msg.user else {
                    continue;
                };
                let Some(ref text) = msg.text else {
                    continue;
                };

                let user_name = slack
                    .get_user_name(user_id)
                    .await
                    .unwrap_or_else(|_| user_id.clone());

                batch.push(SlackMessage {
                    text: text.clone(),
                    user_id: user_id.clone(),
                    user_name,
                    channel_id: channel_id.clone(),
                    channel_name: channel_name.clone(),
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
                        let reply_user_name = slack
                            .get_user_name(reply_user)
                            .await
                            .unwrap_or_else(|_| reply_user.clone());

                        batch.push(SlackMessage {
                            text: reply_text.clone(),
                            user_id: reply_user.clone(),
                            user_name: reply_user_name,
                            channel_id: channel_id.clone(),
                            channel_name: channel_name.clone(),
                            ts: reply.ts.clone(),
                            thread_ts: reply.thread_ts.clone(),
                        });
                    }
                }

                last_ts = Some(msg.ts.clone());
            }

            batch.sort_by(|a, b| a.ts.cmp(&b.ts));

            for chunk in batch.chunks(config.ingest_batch_size) {
                let ingest_msgs: Vec<_> = chunk
                    .iter()
                    .map(|m| slack_to_ingest(m, "slack-ingester"))
                    .collect();
                let count = vegapunk.ingest(ingest_msgs, "slack-ingester").await?;
                info!(channel_id, ingested = count, "batch ingested");
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
