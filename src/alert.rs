use anyhow::Result;
use reqwest::Client;
use tracing::{error, info};

/// Client for sending alert notifications to a Slack channel.
pub struct AlertClient {
    http: Client,
    bot_token: String,
    channel_id: Option<String>,
}

impl AlertClient {
    /// Create a new alert client. If `channel_id` is `None`, alerts are silently dropped.
    pub fn new(bot_token: &str, channel_id: Option<String>) -> Self {
        Self {
            http: Client::new(),
            bot_token: bot_token.to_string(),
            channel_id,
        }
    }

    /// Send an alert message. No-ops if no alert channel is configured.
    pub async fn send(&self, message: &str) -> Result<()> {
        let Some(ref channel_id) = self.channel_id else {
            return Ok(());
        };

        let resp = self
            .http
            .post("https://slack.com/api/chat.postMessage")
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .json(&serde_json::json!({
                "channel": channel_id,
                "text": format!(":rotating_light: *vegapunk-slack-ingester alert*\n{message}"),
            }))
            .send()
            .await;

        match resp {
            Ok(_) => info!("alert sent to {channel_id}"),
            Err(e) => error!("failed to send alert: {e}"),
        }

        Ok(())
    }
}
