use anyhow::{Context, Result};
use chrono::NaiveDate;
use tracing::info;

use crate::config::Config;
use crate::converter::{slack_to_ingest, SlackMessage};
use crate::slack::SlackClient;
use crate::vegapunk::VegapunkClient;

/// Import historical messages for a single channel starting from `since` (YYYY-MM-DD).
pub async fn run_import(
    config: &Config,
    slack: &mut SlackClient,
    vegapunk: &mut VegapunkClient,
    channel_id: &str,
    since: &str,
) -> Result<()> {
    let since_date = NaiveDate::parse_from_str(since, "%Y-%m-%d")
        .with_context(|| format!("invalid date format: {since}. Expected YYYY-MM-DD"))?;
    let since_ts = since_date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let oldest = format!("{since_ts}.000000");

    let channel_name = slack
        .get_channel_name(channel_id)
        .await
        .unwrap_or_else(|_| channel_id.to_string());

    info!(channel_id, channel_name, since, "starting import");

    let mut total_count: u32 = 0;
    let mut cursor_page: Option<String> = None;

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
                channel_id: channel_id.to_string(),
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
                        channel_id: channel_id.to_string(),
                        channel_name: channel_name.clone(),
                        ts: reply.ts.clone(),
                        thread_ts: reply.thread_ts.clone(),
                    });
                }

                tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            }
        }

        batch.sort_by(|a, b| a.ts.cmp(&b.ts));

        for chunk in batch.chunks(config.ingest_batch_size) {
            let ingest_msgs: Vec<_> = chunk
                .iter()
                .map(|m| slack_to_ingest(m, "slack-ingester"))
                .collect();
            let count = vegapunk.ingest(ingest_msgs, "slack-ingester").await?;
            total_count += count as u32;
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

    info!(channel_id, total_count, "import complete");
    Ok(())
}
