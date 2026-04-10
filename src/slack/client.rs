use anyhow::{bail, Context, Result};
use reqwest::Client;
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

use super::types::*;
use crate::cache::TtlCache;

/// HTTP client for the Slack Web API with built-in rate limiting and caching.
pub struct SlackClient {
    http: Client,
    bot_token: String,
    user_cache: TtlCache,
    channel_cache: TtlCache,
}

impl SlackClient {
    /// Create a new client. `cache_ttl_secs` controls how long user/channel names are cached.
    pub fn new(bot_token: &str, cache_ttl_secs: u64) -> Self {
        Self {
            http: Client::new(),
            bot_token: bot_token.to_string(),
            user_cache: TtlCache::new(cache_ttl_secs),
            channel_cache: TtlCache::new(cache_ttl_secs),
        }
    }

    /// Resolve a user ID to a display name, with caching.
    pub async fn get_user_name(&mut self, user_id: &str) -> Result<String> {
        if let Some(name) = self.user_cache.get(user_id) {
            return Ok(name.to_string());
        }

        let resp: SlackApiResponse<UserInfoData> =
            self.api_get("users.info", &[("user", user_id)]).await?;

        let user = resp.data.context("missing user data")?.user;
        let name = user
            .profile
            .display_name
            .filter(|n| !n.is_empty())
            .or(user.real_name)
            .unwrap_or_else(|| user_id.to_string());

        self.user_cache.set(user_id.to_string(), name.clone());
        Ok(name)
    }

    /// Resolve a channel ID to a name (e.g. `#general`), with caching.
    pub async fn get_channel_name(&mut self, channel_id: &str) -> Result<String> {
        if let Some(name) = self.channel_cache.get(channel_id) {
            return Ok(name.to_string());
        }

        let resp: SlackApiResponse<ConversationInfoData> = self
            .api_get("conversations.info", &[("channel", channel_id)])
            .await?;

        let channel = resp.data.context("missing channel data")?.channel;
        let name = channel
            .name
            .map(|n| format!("#{n}"))
            .unwrap_or_else(|| channel_id.to_string());

        self.channel_cache.set(channel_id.to_string(), name.clone());
        Ok(name)
    }

    /// Fetch a page of channel history, optionally filtered by `oldest` timestamp and paginated via `cursor`.
    pub async fn conversations_history(
        &self,
        channel_id: &str,
        oldest: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<HistoryData> {
        let mut params: Vec<(&str, &str)> = vec![("channel", channel_id)];
        let limit_str = limit.to_string();
        params.push(("limit", &limit_str));
        if let Some(oldest) = oldest {
            params.push(("oldest", oldest));
        }
        if let Some(cursor) = cursor {
            params.push(("cursor", cursor));
        }

        let resp: SlackApiResponse<HistoryData> =
            self.api_get("conversations.history", &params).await?;
        resp.data.context("missing history data")
    }

    /// Fetch all replies in a thread.
    pub async fn conversations_replies(
        &self,
        channel_id: &str,
        thread_ts: &str,
    ) -> Result<HistoryData> {
        let resp: SlackApiResponse<HistoryData> = self
            .api_get(
                "conversations.replies",
                &[("channel", channel_id), ("ts", thread_ts)],
            )
            .await?;
        resp.data.context("missing replies data")
    }

    async fn api_get<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: &[(&str, &str)],
    ) -> Result<SlackApiResponse<T>> {
        let url = format!("https://slack.com/api/{method}");

        loop {
            let resp = self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.bot_token))
                .query(params)
                .send()
                .await
                .with_context(|| format!("failed to call Slack API: {method}"))?;

            if resp.status() == 429 {
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(30);
                warn!(method, retry_after, "Slack API rate limited, waiting");
                sleep(Duration::from_secs(retry_after)).await;
                continue;
            }

            let api_resp: SlackApiResponse<T> = resp.json().await?;
            if !api_resp.ok {
                bail!(
                    "Slack API error in {}: {}",
                    method,
                    api_resp.error.unwrap_or_else(|| "unknown".to_string())
                );
            }
            return Ok(api_resp);
        }
    }
}
