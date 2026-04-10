use anyhow::{Context, Result};

/// Configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    // Required
    pub slack_bot_token: String,
    pub slack_app_token: String,
    pub vegapunk_auth_token: String,
    // Optional with defaults
    pub vegapunk_grpc_endpoint: String,
    pub slack_watch_channel_ids: Vec<String>,
    pub slack_alert_channel_id: Option<String>,
    pub ingest_batch_size: usize,
    pub sort_buffer_window_secs: u64,
    pub cursor_file_path: String,
}

impl Config {
    /// Load and validate config from environment variables.
    pub fn from_env() -> Result<Self> {
        let slack_bot_token = required_env("SLACK_BOT_TOKEN")?;
        let slack_app_token = required_env("SLACK_APP_TOKEN")?;
        let vegapunk_auth_token = required_env("VEGAPUNK_AUTH_TOKEN")?;

        let vegapunk_grpc_endpoint = std::env::var("VEGAPUNK_GRPC_ENDPOINT")
            .unwrap_or_else(|_| "host.docker.internal:6840".to_string());

        let slack_watch_channel_ids = std::env::var("SLACK_WATCH_CHANNEL_IDS")
            .map(|s| {
                s.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let slack_alert_channel_id = std::env::var("SLACK_ALERT_CHANNEL_ID").ok();

        let ingest_batch_size = std::env::var("INGEST_BATCH_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20);

        let sort_buffer_window_secs = std::env::var("SORT_BUFFER_WINDOW_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5);

        let cursor_file_path = std::env::var("CURSOR_FILE_PATH")
            .unwrap_or_else(|_| "/app/data/cursor.json".to_string());

        Ok(Config {
            slack_bot_token,
            slack_app_token,
            vegapunk_auth_token,
            vegapunk_grpc_endpoint,
            slack_watch_channel_ids,
            slack_alert_channel_id,
            ingest_batch_size,
            sort_buffer_window_secs,
            cursor_file_path,
        })
    }
}

fn required_env(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("required environment variable {key} is not set"))
}
