use crate::converter::SlackMessage;

/// In-memory buffer that accumulates Slack messages and flushes them
/// in time-sorted order with parent messages guaranteed to precede their replies.
pub struct SortBuffer {
    messages: Vec<SlackMessage>,
}

impl Default for SortBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl SortBuffer {
    /// Create a new, empty sort buffer.
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    /// Add a message to the buffer.
    pub fn push(&mut self, msg: SlackMessage) {
        self.messages.push(msg);
    }

    /// Number of messages currently buffered.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Flush all buffered messages, sorted by ts with parent messages before their replies.
    pub fn flush(&mut self) -> Vec<SlackMessage> {
        let mut msgs = std::mem::take(&mut self.messages);

        // Primary sort by ts ascending.
        msgs.sort_by(|a, b| a.ts.cmp(&b.ts));

        // Ensure parent messages come before their replies.
        let mut result: Vec<SlackMessage> = Vec::with_capacity(msgs.len());
        let mut deferred: Vec<SlackMessage> = Vec::new();

        for msg in msgs {
            if let Some(ref parent_ts) = msg.thread_ts {
                let parent_in_result = result.iter().any(|m| m.ts == *parent_ts);
                if parent_in_result {
                    result.push(msg);
                } else {
                    deferred.push(msg);
                }
            } else {
                result.push(msg);
                // SAFETY: we just pushed a message so result is non-empty.
                let parent_ts = result.last().unwrap().ts.clone();
                let (matched, remaining): (Vec<_>, Vec<_>) = deferred
                    .into_iter()
                    .partition(|m| m.thread_ts.as_deref() == Some(&parent_ts));
                result.extend(matched);
                deferred = remaining;
            }
        }

        // Append deferred replies whose parent is not in this buffer (already ingested).
        result.extend(deferred);

        result
    }
}
