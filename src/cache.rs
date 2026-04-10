use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Simple in-memory cache with TTL-based expiry.
pub struct TtlCache {
    entries: HashMap<String, (String, Instant)>,
    ttl: Duration,
}

impl TtlCache {
    /// Create a new cache with the given TTL in seconds.
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: HashMap::new(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Get a value if it exists and has not expired.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).and_then(|(value, inserted_at)| {
            if inserted_at.elapsed() < self.ttl {
                Some(value.as_str())
            } else {
                None
            }
        })
    }

    /// Insert or update a key-value pair, resetting its TTL.
    pub fn set(&mut self, key: String, value: String) {
        self.entries.insert(key, (value, Instant::now()));
    }
}
