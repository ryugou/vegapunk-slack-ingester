use vegapunk_slack_ingester::converter::{slack_to_ingest, SlackMessage};

#[test]
fn test_basic_message_conversion() {
    let msg = SlackMessage {
        text: "Hello world".to_string(),
        user_id: "U01ABC123".to_string(),
        user_name: "山田太郎".to_string(),
        channel_id: "C06ABC123".to_string(),
        channel_name: "#engineering".to_string(),
        ts: "1712345678.123456".to_string(),
        thread_ts: None,
    };

    let result = slack_to_ingest(&msg);

    assert_eq!(result.id, "C06ABC123-1712345678.123456");
    assert_eq!(result.text, "Hello world");
    assert_eq!(result.metadata.source_type, "slack");
    assert_eq!(result.metadata.author, "山田太郎");
    assert_eq!(result.metadata.author_id, "U01ABC123");
    assert_eq!(result.metadata.channel, "#engineering");
    assert_eq!(result.metadata.channel_id, "C06ABC123");
    // Parent messages (thread_ts=None) use their own ts as thread_id
    assert_eq!(
        result.metadata.thread_id.as_deref(),
        Some("1712345678.123456")
    );
    assert!(result.metadata.timestamp.contains("T"));
    assert!(result.metadata.timestamp.contains("+") || result.metadata.timestamp.contains("Z"));
}

#[test]
fn test_thread_reply_conversion() {
    let msg = SlackMessage {
        text: "Reply in thread".to_string(),
        user_id: "U01ABC123".to_string(),
        user_name: "山田太郎".to_string(),
        channel_id: "C06ABC123".to_string(),
        channel_name: "#engineering".to_string(),
        ts: "1712345700.654321".to_string(),
        thread_ts: Some("1712345678.123456".to_string()),
    };

    let result = slack_to_ingest(&msg);

    assert_eq!(
        result.metadata.thread_id.as_deref(),
        Some("1712345678.123456")
    );
}

#[test]
fn test_timestamp_conversion() {
    let msg = SlackMessage {
        text: "test".to_string(),
        user_id: "U01".to_string(),
        user_name: "test".to_string(),
        channel_id: "C01".to_string(),
        channel_name: "#test".to_string(),
        ts: "1712345678.123456".to_string(),
        thread_ts: None,
    };

    let result = slack_to_ingest(&msg);

    let parsed = chrono::DateTime::parse_from_rfc3339(&result.metadata.timestamp);
    assert!(
        parsed.is_ok(),
        "timestamp must be valid RFC3339: {}",
        result.metadata.timestamp
    );
}
