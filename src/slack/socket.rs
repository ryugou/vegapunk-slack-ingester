use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tracing::{error, info, warn};

use crate::alert::AlertClient;
use crate::buffer::SortBuffer;
use crate::config::Config;
use crate::converter::{ingest_batch, SlackMessage};
use crate::cursor::CursorStore;
use crate::slack::SlackClient;
use crate::vegapunk::VegapunkClient;

// ---------------------------------------------------------------------------
// Wire types — private to this module
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct SocketEnvelope {
    envelope_id: Option<String>,
    #[serde(rename = "type")]
    event_type: String,
    payload: Option<EventPayload>,
}

#[derive(Debug, serde::Deserialize)]
struct EventPayload {
    event: Option<SocketEvent>,
}

#[derive(Debug, serde::Deserialize)]
struct SocketEvent {
    #[serde(rename = "type")]
    event_type: String,
    text: Option<String>,
    user: Option<String>,
    channel: Option<String>,
    ts: String,
    thread_ts: Option<String>,
    bot_id: Option<String>,
    files: Option<Vec<crate::slack::types::SlackFile>>,
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Returns `true` when `channel_id` should be ingested.
///
/// An empty `watched` list is interpreted as "watch everything".
fn is_watched_channel(channel_id: &str, watched: &[String]) -> bool {
    watched.is_empty() || watched.iter().any(|id| id == channel_id)
}

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

/// Call `apps.connections.open` and return the WebSocket URL.
async fn fetch_ws_url(app_token: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;
    let response = client
        .post("https://slack.com/api/apps.connections.open")
        .header("Authorization", format!("Bearer {app_token}"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send()
        .await
        .context("failed to POST apps.connections.open")?;

    let body: serde_json::Value = response
        .json()
        .await
        .context("failed to parse apps.connections.open response as JSON")?;

    let ok = body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !ok {
        let slack_err = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        anyhow::bail!("apps.connections.open returned ok=false: {slack_err}");
    }

    body.get("url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .context("apps.connections.open response missing 'url' field")
}

/// Resolve a `SocketEvent` into a `SlackMessage`, or return `None` if the
/// event should be skipped (bot message, missing fields, unwatched channel).
async fn build_message_from_event(
    event: SocketEvent,
    config: &Config,
    slack: &mut SlackClient,
) -> Result<Option<SlackMessage>> {
    if event.bot_id.is_some() {
        return Ok(None);
    }

    let text = match event.text {
        Some(t) if !t.is_empty() => t,
        _ => return Ok(None),
    };
    let user_id = match event.user {
        Some(u) => u,
        None => return Ok(None),
    };
    let channel_id = match event.channel {
        Some(c) => c,
        None => return Ok(None),
    };

    if !is_watched_channel(&channel_id, &config.slack_watch_channel_ids) {
        return Ok(None);
    }

    let user_name = slack
        .get_user_name(&user_id)
        .await
        .unwrap_or_else(|_| user_id.clone());

    let channel_name = slack
        .get_channel_name(&channel_id)
        .await
        .unwrap_or_else(|_| channel_id.clone());

    let text =
        crate::extractor::enrich_text(&text, event.files.as_deref(), &config.slack_user_token)
            .await;

    Ok(Some(SlackMessage {
        text,
        user_id,
        user_name,
        channel_id,
        channel_name,
        ts: event.ts,
        thread_ts: event.thread_ts,
    }))
}

/// Flush the sort buffer and ingest each channel's messages into Vegapunk,
/// then advance the per-channel cursor to the latest ts seen.
///
/// On failure, un-ingested messages are pushed back into the buffer so they
/// are retried on the next flush cycle instead of being silently dropped.
async fn flush_and_ingest(
    buffer: &mut SortBuffer,
    vegapunk: &mut VegapunkClient,
    cursor_store: &CursorStore,
    config: &Config,
) -> Result<()> {
    if buffer.is_empty() {
        return Ok(());
    }

    let flushed: Vec<SlackMessage> = buffer.flush();

    // Collect unique channel IDs while preserving encounter order.
    let mut channel_ids: Vec<String> = Vec::new();
    for msg in &flushed {
        if !channel_ids.contains(&msg.channel_id) {
            channel_ids.push(msg.channel_id.clone());
        }
    }

    for channel_id in &channel_ids {
        let mut channel_msgs: Vec<SlackMessage> = flushed
            .iter()
            .filter(|m| m.channel_id == *channel_id)
            .cloned()
            .collect();

        match ingest_batch(
            &mut channel_msgs,
            vegapunk,
            config.ingest_batch_size,
            channel_id,
        )
        .await
        {
            Ok(_) => {
                // Cursor advances to the lexicographically largest ts in this batch
                if let Some(latest_ts) = channel_msgs.iter().map(|m| m.ts.as_str()).max() {
                    if let Err(e) = cursor_store.save_channel(channel_id, latest_ts) {
                        warn!(channel_id, error = %e, "failed to save cursor");
                    }
                }
            }
            Err(e) => {
                // Push failed messages back into the buffer for retry
                warn!(channel_id, error = %e, "ingest failed, returning messages to buffer");
                for msg in channel_msgs {
                    buffer.push(msg);
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Core event loop
// ---------------------------------------------------------------------------

/// Connect to Slack via Socket Mode and process events until an error or
/// SIGINT.  On clean SIGINT the remaining buffer is flushed before returning.
async fn run_socket_loop(
    config: &Config,
    slack: &mut SlackClient,
    vegapunk: &mut VegapunkClient,
    cursor_store: &CursorStore,
) -> Result<()> {
    let ws_url = fetch_ws_url(&config.slack_app_token)
        .await
        .context("failed to obtain WebSocket URL from Slack")?;

    let (ws_stream, _response) =
        tokio::time::timeout(Duration::from_secs(30), connect_async(&ws_url))
            .await
            .context("WebSocket connection timed out")?
            .context("failed to connect to Slack WebSocket")?;

    let (mut ws_write, mut ws_read) = ws_stream.split();

    // The first message must be a "hello" acknowledgement from Slack.
    let hello_raw = ws_read
        .next()
        .await
        .context("WebSocket closed before hello")?
        .context("error reading hello message")?;

    let hello_text = match hello_raw {
        WsMessage::Text(t) => t,
        other => anyhow::bail!("expected text hello, got {:?}", other),
    };

    let hello: SocketEnvelope =
        serde_json::from_str(hello_text.as_str()).context("failed to parse hello envelope")?;
    if hello.event_type != "hello" {
        anyhow::bail!("expected hello envelope, got type={}", hello.event_type);
    }
    info!("Socket Mode connected — received hello");

    let mut buffer = SortBuffer::new();
    let mut flush_interval =
        tokio::time::interval(Duration::from_secs(config.sort_buffer_window_secs));
    // Consume the immediate first tick so the flush only fires after a full window.
    flush_interval.tick().await;

    loop {
        tokio::select! {
            raw_msg = ws_read.next() => {
                match raw_msg {
                    None => anyhow::bail!("WebSocket stream closed unexpectedly"),
                    Some(Err(e)) => {
                        return Err(anyhow::anyhow!(e).context("WebSocket read error"));
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        ws_write
                            .send(WsMessage::Pong(data))
                            .await
                            .context("failed to send Pong")?;
                    }
                    Some(Ok(WsMessage::Text(text))) => {
                        let envelope: SocketEnvelope = match serde_json::from_str(text.as_str()) {
                            Ok(e) => e,
                            Err(err) => {
                                warn!("failed to parse socket envelope: {err}");
                                continue;
                            }
                        };

                        if envelope.event_type == "events_api" {
                            // Acknowledge immediately.
                            if let Some(ref eid) = envelope.envelope_id {
                                let ack = serde_json::json!({"envelope_id": eid}).to_string();
                                ws_write
                                    .send(WsMessage::Text(ack))
                                    .await
                                    .context("failed to send ACK")?;
                            }

                            if let Some(payload) = envelope.payload {
                                if let Some(event) = payload.event {
                                    if event.event_type == "message" {
                                        match build_message_from_event(event, config, slack).await {
                                            Ok(Some(msg)) => {
                                                info!(
                                                    channel_id = %msg.channel_id,
                                                    ts = %msg.ts,
                                                    "buffering message"
                                                );
                                                buffer.push(msg);
                                            }
                                            Ok(None) => {} // skipped (bot / unwatched / missing fields)
                                            Err(e) => {
                                                warn!("failed to build message from event: {e:#}");
                                            }
                                        }
                                    }
                                }
                            }
                        } else if envelope.event_type == "disconnect" {
                            anyhow::bail!("Slack requested disconnect");
                        }
                        // Other envelope types (e.g. "hello" replays) are silently ignored.
                    }
                    Some(Ok(_)) => {} // ignore binary / close frames
                }
            }

            _ = flush_interval.tick() => {
                if !buffer.is_empty() {
                    if let Err(e) = flush_and_ingest(&mut buffer, vegapunk, cursor_store, config).await {
                        warn!("flush_and_ingest failed: {e:#}");
                    }
                }
            }

            _ = tokio::signal::ctrl_c() => {
                info!("SIGINT received — flushing buffer before shutdown");
                if let Err(e) = flush_and_ingest(&mut buffer, vegapunk, cursor_store, config).await {
                    warn!("final flush failed: {e:#}");
                }
                return Ok(());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run Socket Mode with automatic reconnection on failure.
///
/// On a clean SIGINT the function returns `Ok(())`.  On any connection error
/// an alert is sent and the loop retries after a 5-second back-off; a SIGINT
/// during that delay also causes a clean return.
pub async fn run_socket_mode(
    config: &Config,
    slack: &mut SlackClient,
    vegapunk: &mut VegapunkClient,
    cursor_store: &CursorStore,
    alert: &AlertClient,
) -> Result<()> {
    loop {
        match run_socket_loop(config, slack, vegapunk, cursor_store).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                error!("Socket Mode error: {e:#}");
                let _ = alert
                    .send(&format!("Socket Mode disconnected, reconnecting: {e:#}"))
                    .await;

                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    _ = tokio::signal::ctrl_c() => {
                        info!("SIGINT received during reconnect delay — shutting down");
                        return Ok(());
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Parsing tests -------------------------------------------------------

    #[test]
    fn test_parse_hello_envelope() {
        let json = r#"{"type":"hello","num_connections":1,"debug_info":{},"connection_info":{"app_id":"A123"}}"#;
        let envelope: SocketEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.event_type, "hello");
        assert!(envelope.envelope_id.is_none());
        assert!(envelope.payload.is_none());
    }

    #[test]
    fn test_parse_events_api_envelope() {
        let json = r#"{
            "envelope_id": "eid-001",
            "type": "events_api",
            "payload": {
                "event": {
                    "type": "message",
                    "text": "hello world",
                    "user": "U001",
                    "channel": "C001",
                    "ts": "1712345678.000001"
                }
            }
        }"#;
        let envelope: SocketEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.event_type, "events_api");
        assert_eq!(envelope.envelope_id.as_deref(), Some("eid-001"));

        let event = envelope.payload.unwrap().event.unwrap();
        assert_eq!(event.event_type, "message");
        assert_eq!(event.text.as_deref(), Some("hello world"));
        assert_eq!(event.user.as_deref(), Some("U001"));
        assert_eq!(event.channel.as_deref(), Some("C001"));
        assert_eq!(event.ts, "1712345678.000001");
        assert!(event.bot_id.is_none());
        assert!(event.thread_ts.is_none());
    }

    #[test]
    fn test_parse_bot_message_has_bot_id() {
        let json = r#"{
            "envelope_id": "eid-002",
            "type": "events_api",
            "payload": {
                "event": {
                    "type": "message",
                    "text": "bot says hi",
                    "bot_id": "B001",
                    "channel": "C001",
                    "ts": "1712345679.000001"
                }
            }
        }"#;
        let envelope: SocketEnvelope = serde_json::from_str(json).unwrap();
        let event = envelope.payload.unwrap().event.unwrap();
        assert!(event.bot_id.is_some());
        assert_eq!(event.bot_id.as_deref(), Some("B001"));
    }

    #[test]
    fn test_parse_thread_reply_has_thread_ts() {
        let json = r#"{
            "envelope_id": "eid-003",
            "type": "events_api",
            "payload": {
                "event": {
                    "type": "message",
                    "text": "reply in thread",
                    "user": "U002",
                    "channel": "C002",
                    "ts": "1712345680.000001",
                    "thread_ts": "1712345670.000001"
                }
            }
        }"#;
        let envelope: SocketEnvelope = serde_json::from_str(json).unwrap();
        let event = envelope.payload.unwrap().event.unwrap();
        assert_eq!(event.thread_ts.as_deref(), Some("1712345670.000001"));
        assert_eq!(event.ts, "1712345680.000001");
    }

    // --- is_watched_channel tests --------------------------------------------

    #[test]
    fn test_is_watched_channel_empty_watchlist_allows_all() {
        assert!(is_watched_channel("C_ANY", &[]));
        assert!(is_watched_channel("C_OTHER", &[]));
    }

    #[test]
    fn test_is_watched_channel_allows_known_channels() {
        let watched = vec!["C001".to_string(), "C002".to_string()];
        assert!(is_watched_channel("C001", &watched));
        assert!(is_watched_channel("C002", &watched));
    }

    #[test]
    fn test_is_watched_channel_blocks_unknown_channels() {
        let watched = vec!["C001".to_string(), "C002".to_string()];
        assert!(!is_watched_channel("C999", &watched));
        assert!(!is_watched_channel("", &watched));
    }
}
