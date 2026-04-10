use anyhow::Result;
use reqwest::Client;
use std::time::Duration;
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
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client"); // infallible with default TLS
        Self {
            http,
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
            Ok(r) => {
                let body: serde_json::Value = r.json().await.unwrap_or_default();
                if body.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                    info!("alert sent to {channel_id}");
                } else {
                    let err = body
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    error!(channel_id, error = err, "Slack rejected alert message");
                }
            }
            Err(e) => error!("failed to send alert: {e}"),
        }

        Ok(())
    }
}
