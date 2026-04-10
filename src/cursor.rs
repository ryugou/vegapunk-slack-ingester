use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;

/// Persistent store for per-channel ingest cursor (last ingested message ts).
pub struct CursorStore {
    path: PathBuf,
}

impl CursorStore {
    /// Create a new cursor store backed by a JSON file at the given path.
    pub fn new(path: &str) -> Self {
        Self {
            path: PathBuf::from(path),
        }
    }

    /// Load all channel cursors. Returns empty map if file does not exist.
    pub fn load(&self) -> Result<HashMap<String, String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => {
                let cursors: HashMap<String, String> = serde_json::from_str(&content)
                    .with_context(|| "failed to parse cursor file")?;
                Ok(cursors)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(anyhow::anyhow!(e).context(format!(
                "failed to read cursor file: {}",
                self.path.display()
            ))),
        }
    }

    /// Update the cursor for a single channel and persist to disk.
    pub fn save_channel(&self, channel_id: &str, ts: &str) -> Result<()> {
        let mut cursors = self.load()?;
        cursors.insert(channel_id.to_string(), ts.to_string());
        self.write(&cursors)
    }

    fn write(&self, cursors: &HashMap<String, String>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create cursor directory: {}", parent.display())
            })?;
        }
        let content = serde_json::to_string_pretty(cursors)?;
        std::fs::write(&self.path, content)
            .with_context(|| format!("failed to write cursor file: {}", self.path.display()))?;
        Ok(())
    }
}
