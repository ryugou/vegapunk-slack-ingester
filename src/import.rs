use anyhow::{Context, Result};
use chrono::NaiveDate;
use tracing::info;

use crate::config::Config;
use crate::converter::{history_to_slack_messages, ingest_batch};
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
        .with_context(|| format!("failed to create datetime from date: {since}"))?
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

        let mut batch = history_to_slack_messages(
            &history.messages,
            channel_id,
            &channel_name,
            slack,
            &config.slack_user_token,
        )
        .await?;

        total_count +=
            ingest_batch(&mut batch, vegapunk, config.ingest_batch_size, channel_id).await?;

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

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    }

    info!(channel_id, total_count, "import complete");
    Ok(())
}
