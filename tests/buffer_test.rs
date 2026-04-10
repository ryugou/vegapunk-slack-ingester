use vegapunk_slack_ingester::buffer::SortBuffer;
use vegapunk_slack_ingester::converter::SlackMessage;

fn make_msg(ts: &str, thread_ts: Option<&str>) -> SlackMessage {
    SlackMessage {
        text: format!("msg at {ts}"),
        user_id: "U01".to_string(),
        user_name: "test".to_string(),
        channel_id: "C01".to_string(),
        channel_name: "#test".to_string(),
        ts: ts.to_string(),
        thread_ts: thread_ts.map(|s| s.to_string()),
    }
}

#[test]
fn test_flush_sorts_by_ts() {
    let mut buffer = SortBuffer::new();

    buffer.push(make_msg("1712345700.000000", None));
    buffer.push(make_msg("1712345678.000000", None));
    buffer.push(make_msg("1712345690.000000", None));

    let flushed = buffer.flush();

    assert_eq!(flushed.len(), 3);
    assert_eq!(flushed[0].ts, "1712345678.000000");
    assert_eq!(flushed[1].ts, "1712345690.000000");
    assert_eq!(flushed[2].ts, "1712345700.000000");
}

#[test]
fn test_flush_parent_before_reply() {
    let mut buffer = SortBuffer::new();

    buffer.push(make_msg("1712345700.000000", Some("1712345678.000000")));
    buffer.push(make_msg("1712345678.000000", None));

    let flushed = buffer.flush();

    assert_eq!(flushed.len(), 2);
    assert_eq!(flushed[0].ts, "1712345678.000000");
    assert!(flushed[0].thread_ts.is_none());
    assert_eq!(flushed[1].ts, "1712345700.000000");
    assert_eq!(flushed[1].thread_ts.as_deref(), Some("1712345678.000000"));
}

#[test]
fn test_flush_empty() {
    let mut buffer = SortBuffer::new();
    let flushed = buffer.flush();
    assert!(flushed.is_empty());
}
