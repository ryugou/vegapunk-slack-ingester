# vegapunk-slack-ingester Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Slack メッセージを Vegapunk の `ingest` API に継続的に投入する Rust 製常駐プロセスを構築する。

**Architecture:** Socket Mode で Slack からリアルタイム受信し、Sort Buffer で順序保証した上で Vegapunk の gRPC `ingest` API にメッセージを投入する。`serve`（常駐）と `import`（一回実行）の 2 サブコマンドを持つ単一バイナリ。

**Tech Stack:** Rust, tonic 0.12 (gRPC), prost 0.13, slack-morphism 2.x (Slack API + Socket Mode), tokio, chrono, tracing, clap, serde

**Design doc:** `docs/superpowers/specs/2026-04-10-slack-ingester-design.md`

**Proto:** `sivira/vegapunk-proto` を git submodule として管理。vegapunk 本体と tonic/prost バージョンを揃える。

---

## File Structure

```
vegapunk-slack-ingester/
├── Cargo.toml
├── Cargo.lock
├── build.rs                 # tonic-build で proto からコード生成
├── Dockerfile
├── docker-compose.yml
├── proto/                   # git submodule (sivira/vegapunk-proto)
│   └── graphrag.proto
├── src/
│   ├── main.rs              # CLI エントリポイント (serve / import / health)
│   ├── config.rs            # 環境変数の読み込み・バリデーション
│   ├── vegapunk/
│   │   ├── mod.rs           # pub mod + re-export
│   │   └── client.rs        # Vegapunk gRPC クライアント (ingest / list_schemas)
│   ├── slack/
│   │   ├── mod.rs           # pub mod + re-export
│   │   ├── client.rs        # Slack Web API クライアント (users.info, conversations.info/history/replies)
│   │   ├── socket.rs        # Socket Mode 接続・イベント受信
│   │   └── types.rs         # Slack API のレスポンス型定義
│   ├── converter.rs         # Slack メッセージ → ingest API 形式の変換
│   ├── buffer.rs            # Sort Buffer (リアルタイム受信時の順序保証)
│   ├── cursor.rs            # ローカルカーソル管理 (JSON ファイル読み書き)
│   ├── catchup.rs           # キャッチアップ処理 (serve 起動時の差分同期)
│   ├── import.rs            # import サブコマンドの処理
│   └── alert.rs             # 障害通知 (Slack チャンネルへの投稿)
└── tests/
    ├── converter_test.rs    # 変換ロジックのユニットテスト
    ├── buffer_test.rs       # Sort Buffer のユニットテスト
    ├── cursor_test.rs       # カーソル管理のユニットテスト
    └── config_test.rs       # 設定バリデーションのユニットテスト
```

---

## Task 1: プロジェクト初期化 + proto セットアップ

**Files:**
- Create: `Cargo.toml`
- Create: `build.rs`
- Create: `src/main.rs`
- Create: `rust-toolchain.toml`

- [ ] **Step 1: Cargo.toml を作成**

```toml
[package]
name = "vegapunk-slack-ingester"
version = "0.1.0"
edition = "2021"

[dependencies]
tonic = { version = "0.12", features = ["transport"] }
prost = "0.13"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4", features = ["derive"] }
reqwest = { version = "0.12", features = ["json"] }
anyhow = "1"
thiserror = "1"
slack-morphism = { version = "2", features = ["hyper", "axum"] }

[build-dependencies]
tonic-build = "0.12"
```

- [ ] **Step 2: proto サブモジュールを追加**

Run:
```bash
git submodule add git@github.com:sivira/vegapunk-proto.git proto
```

Expected: `proto/graphrag.proto` が存在する

- [ ] **Step 3: build.rs を作成**

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic::build::configure()
        .build_server(false)
        .compile_protos(&["proto/graphrag.proto"], &["proto"])?;
    Ok(())
}
```

- [ ] **Step 4: rust-toolchain.toml を作成**

```toml
[toolchain]
channel = "stable"
```

- [ ] **Step 5: 最小の main.rs を作成**

```rust
fn main() {
    println!("vegapunk-slack-ingester");
}
```

- [ ] **Step 6: ビルド確認**

Run: `cargo build`
Expected: proto コード生成が走り、ビルドが成功する

- [ ] **Step 7: コミット**

```bash
git add Cargo.toml Cargo.lock build.rs rust-toolchain.toml src/main.rs .gitmodules proto
git commit -m "feat: initialize project with proto submodule and dependencies"
```

---

## Task 2: 設定管理 (`config.rs`)

**Files:**
- Create: `src/config.rs`
- Create: `tests/config_test.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: config のテストを書く**

```rust
// tests/config_test.rs
use vegapunk_slack_ingester::config::Config;

#[test]
fn test_config_from_env_all_required() {
    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
    std::env::set_var("SLACK_APP_TOKEN", "xapp-test");
    std::env::set_var("VEGAPUNK_AUTH_TOKEN", "test-token");

    let config = Config::from_env().unwrap();

    assert_eq!(config.slack_bot_token, "xoxb-test");
    assert_eq!(config.slack_app_token, "xapp-test");
    assert_eq!(config.vegapunk_auth_token, "test-token");
    assert_eq!(config.vegapunk_grpc_endpoint, "host.docker.internal:6840");
    assert_eq!(config.ingest_batch_size, 20);
    assert_eq!(config.sort_buffer_window_secs, 5);

    // cleanup
    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");
    std::env::remove_var("VEGAPUNK_AUTH_TOKEN");
}

#[test]
fn test_config_missing_required() {
    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");
    std::env::remove_var("VEGAPUNK_AUTH_TOKEN");

    let result = Config::from_env();
    assert!(result.is_err());
}

#[test]
fn test_config_custom_values() {
    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
    std::env::set_var("SLACK_APP_TOKEN", "xapp-test");
    std::env::set_var("VEGAPUNK_AUTH_TOKEN", "test-token");
    std::env::set_var("VEGAPUNK_GRPC_ENDPOINT", "localhost:6840");
    std::env::set_var("INGEST_BATCH_SIZE", "50");
    std::env::set_var("SORT_BUFFER_WINDOW_SECS", "10");
    std::env::set_var("SLACK_WATCH_CHANNEL_IDS", "C123,C456");

    let config = Config::from_env().unwrap();

    assert_eq!(config.vegapunk_grpc_endpoint, "localhost:6840");
    assert_eq!(config.ingest_batch_size, 50);
    assert_eq!(config.sort_buffer_window_secs, 10);
    assert_eq!(config.slack_watch_channel_ids, vec!["C123", "C456"]);

    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");
    std::env::remove_var("VEGAPUNK_AUTH_TOKEN");
    std::env::remove_var("VEGAPUNK_GRPC_ENDPOINT");
    std::env::remove_var("INGEST_BATCH_SIZE");
    std::env::remove_var("SORT_BUFFER_WINDOW_SECS");
    std::env::remove_var("SLACK_WATCH_CHANNEL_IDS");
}
```

- [ ] **Step 2: テスト失敗を確認**

Run: `cargo test --test config_test`
Expected: FAIL（`config` モジュールが存在しない）

- [ ] **Step 3: config.rs を実装**

```rust
// src/config.rs
use anyhow::{Context, Result};

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
    pub fn from_env() -> Result<Self> {
        let slack_bot_token = required_env("SLACK_BOT_TOKEN")?;
        let slack_app_token = required_env("SLACK_APP_TOKEN")?;
        let vegapunk_auth_token = required_env("VEGAPUNK_AUTH_TOKEN")?;

        let vegapunk_grpc_endpoint = std::env::var("VEGAPUNK_GRPC_ENDPOINT")
            .unwrap_or_else(|_| "host.docker.internal:6840".to_string());

        let slack_watch_channel_ids = std::env::var("SLACK_WATCH_CHANNEL_IDS")
            .map(|s| s.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
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
```

- [ ] **Step 4: main.rs を lib.rs パターンに変更**

```rust
// src/main.rs
pub mod config;

fn main() {
    println!("vegapunk-slack-ingester");
}
```

Cargo.toml に追加:
```toml
[lib]
name = "vegapunk_slack_ingester"
path = "src/main.rs"

[[bin]]
name = "vegapunk-slack-ingester"
path = "src/main.rs"
```

- [ ] **Step 5: テスト成功を確認**

Run: `cargo test --test config_test`
Expected: 3 tests PASS

- [ ] **Step 6: コミット**

```bash
git add src/config.rs src/main.rs tests/config_test.rs Cargo.toml
git commit -m "feat: add config module with env var parsing and validation"
```

---

## Task 3: メッセージ変換 (`converter.rs`)

**Files:**
- Create: `src/converter.rs`
- Create: `tests/converter_test.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: converter のテストを書く**

```rust
// tests/converter_test.rs
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

    let result = slack_to_ingest(&msg, "slack-ingester");

    assert_eq!(result.id, "C06ABC123-1712345678.123456");
    assert_eq!(result.text, "Hello world");
    assert_eq!(result.schema, "slack-ingester");
    assert_eq!(result.metadata.source_type, "slack");
    assert_eq!(result.metadata.author, "山田太郎");
    assert_eq!(result.metadata.author_id, "U01ABC123");
    assert_eq!(result.metadata.channel, "#engineering");
    assert_eq!(result.metadata.channel_id, "C06ABC123");
    assert!(result.metadata.thread_id.is_none());
    // timestamp should be RFC3339
    assert!(result.metadata.timestamp.contains("T"));
    assert!(result.metadata.timestamp.contains("+"));
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

    let result = slack_to_ingest(&msg, "slack-ingester");

    assert_eq!(result.metadata.thread_id.as_deref(), Some("1712345678.123456"));
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

    let result = slack_to_ingest(&msg, "slack-ingester");

    // Unix epoch 1712345678 = 2024-04-05T23:34:38+09:00 (JST)
    // The exact format depends on chrono, but must be RFC3339
    let parsed = chrono::DateTime::parse_from_rfc3339(&result.metadata.timestamp);
    assert!(parsed.is_ok(), "timestamp must be valid RFC3339: {}", result.metadata.timestamp);
}
```

- [ ] **Step 2: テスト失敗を確認**

Run: `cargo test --test converter_test`
Expected: FAIL

- [ ] **Step 3: converter.rs を実装**

```rust
// src/converter.rs
use chrono::{TimeZone, Utc};

#[derive(Debug, Clone)]
pub struct SlackMessage {
    pub text: String,
    pub user_id: String,
    pub user_name: String,
    pub channel_id: String,
    pub channel_name: String,
    pub ts: String,
    pub thread_ts: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IngestMessage {
    pub id: String,
    pub text: String,
    pub metadata: IngestMetadata,
    #[serde(skip)]
    pub schema: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IngestMetadata {
    pub source_type: String,
    pub author: String,
    pub author_id: String,
    pub channel: String,
    pub channel_id: String,
    pub thread_id: Option<String>,
    pub timestamp: String,
}

pub fn slack_to_ingest(msg: &SlackMessage, schema: &str) -> IngestMessage {
    let id = format!("{}-{}", msg.channel_id, msg.ts);
    let timestamp = slack_ts_to_rfc3339(&msg.ts);

    IngestMessage {
        id,
        text: msg.text.clone(),
        schema: schema.to_string(),
        metadata: IngestMetadata {
            source_type: "slack".to_string(),
            author: msg.user_name.clone(),
            author_id: msg.user_id.clone(),
            channel: msg.channel_name.clone(),
            channel_id: msg.channel_id.clone(),
            thread_id: msg.thread_ts.clone(),
            timestamp,
        },
    }
}

fn slack_ts_to_rfc3339(ts: &str) -> String {
    let secs: i64 = ts
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let dt = Utc.timestamp_opt(secs, 0).single().unwrap_or_else(Utc::now);
    dt.to_rfc3339()
}
```

- [ ] **Step 4: main.rs に module を追加**

```rust
pub mod converter;
```

- [ ] **Step 5: テスト成功を確認**

Run: `cargo test --test converter_test`
Expected: 3 tests PASS

- [ ] **Step 6: コミット**

```bash
git add src/converter.rs tests/converter_test.rs src/main.rs
git commit -m "feat: add Slack message to Vegapunk ingest format converter"
```

---

## Task 4: カーソル管理 (`cursor.rs`)

**Files:**
- Create: `src/cursor.rs`
- Create: `tests/cursor_test.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: cursor のテストを書く**

```rust
// tests/cursor_test.rs
use std::collections::HashMap;
use vegapunk_slack_ingester::cursor::CursorStore;

#[test]
fn test_load_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cursor.json");

    let store = CursorStore::new(path.to_str().unwrap());
    let cursors = store.load().unwrap();

    assert!(cursors.is_empty());
}

#[test]
fn test_save_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cursor.json");

    let store = CursorStore::new(path.to_str().unwrap());
    store.save_channel("C123", "1712345678.123456").unwrap();
    store.save_channel("C456", "1712345700.654321").unwrap();

    let cursors = store.load().unwrap();
    assert_eq!(cursors.get("C123"), Some(&"1712345678.123456".to_string()));
    assert_eq!(cursors.get("C456"), Some(&"1712345700.654321".to_string()));
}

#[test]
fn test_update_existing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cursor.json");

    let store = CursorStore::new(path.to_str().unwrap());
    store.save_channel("C123", "1712345678.000000").unwrap();
    store.save_channel("C123", "1712345700.000000").unwrap();

    let cursors = store.load().unwrap();
    assert_eq!(cursors.get("C123"), Some(&"1712345700.000000".to_string()));
}
```

- [ ] **Step 2: テスト失敗を確認**

Run: `cargo test --test cursor_test`
Expected: FAIL

- [ ] **Step 3: Cargo.toml に tempfile 追加**

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 4: cursor.rs を実装**

```rust
// src/cursor.rs
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct CursorStore {
    path: PathBuf,
}

impl CursorStore {
    pub fn new(path: &str) -> Self {
        Self {
            path: PathBuf::from(path),
        }
    }

    pub fn load(&self) -> Result<HashMap<String, String>> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let content = std::fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read cursor file: {}", self.path.display()))?;
        let cursors: HashMap<String, String> = serde_json::from_str(&content)
            .with_context(|| "failed to parse cursor file")?;
        Ok(cursors)
    }

    pub fn save_channel(&self, channel_id: &str, ts: &str) -> Result<()> {
        let mut cursors = self.load().unwrap_or_default();
        cursors.insert(channel_id.to_string(), ts.to_string());
        self.write(&cursors)
    }

    fn write(&self, cursors: &HashMap<String, String>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create cursor directory: {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(cursors)?;
        std::fs::write(&self.path, content)
            .with_context(|| format!("failed to write cursor file: {}", self.path.display()))?;
        Ok(())
    }
}
```

- [ ] **Step 5: main.rs に module を追加**

```rust
pub mod cursor;
```

- [ ] **Step 6: テスト成功を確認**

Run: `cargo test --test cursor_test`
Expected: 3 tests PASS

- [ ] **Step 7: コミット**

```bash
git add src/cursor.rs tests/cursor_test.rs src/main.rs Cargo.toml
git commit -m "feat: add cursor store for tracking per-channel ingest progress"
```

---

## Task 5: Sort Buffer (`buffer.rs`)

**Files:**
- Create: `src/buffer.rs`
- Create: `tests/buffer_test.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: buffer のテストを書く**

```rust
// tests/buffer_test.rs
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

    // Reply arrives before parent
    buffer.push(make_msg("1712345700.000000", Some("1712345678.000000")));
    buffer.push(make_msg("1712345678.000000", None));

    let flushed = buffer.flush();

    assert_eq!(flushed.len(), 2);
    // Parent must come first
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
```

- [ ] **Step 2: テスト失敗を確認**

Run: `cargo test --test buffer_test`
Expected: FAIL

- [ ] **Step 3: buffer.rs を実装**

```rust
// src/buffer.rs
use crate::converter::SlackMessage;

pub struct SortBuffer {
    messages: Vec<SlackMessage>,
}

impl SortBuffer {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    pub fn push(&mut self, msg: SlackMessage) {
        self.messages.push(msg);
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Flush all buffered messages, sorted by ts with parent messages before their replies.
    pub fn flush(&mut self) -> Vec<SlackMessage> {
        let mut msgs = std::mem::take(&mut self.messages);

        // Sort by ts ascending
        msgs.sort_by(|a, b| a.ts.cmp(&b.ts));

        // Ensure parent messages come before their replies.
        // Since we sorted by ts, a parent (with an earlier ts) should already
        // be before its reply. But if a reply has a ts earlier than its parent
        // (unlikely but possible with clock skew), we need to fix the order.
        let mut result: Vec<SlackMessage> = Vec::with_capacity(msgs.len());
        let mut deferred: Vec<SlackMessage> = Vec::new();

        for msg in msgs {
            if let Some(ref parent_ts) = msg.thread_ts {
                // Check if parent is already in result
                let parent_in_result = result.iter().any(|m| m.ts == *parent_ts);
                if parent_in_result {
                    result.push(msg);
                } else {
                    // Parent not yet seen — defer this reply
                    deferred.push(msg);
                }
            } else {
                result.push(msg);
                // Check if any deferred replies belong to this parent
                let parent_ts = &result.last().unwrap().ts;
                let (matched, remaining): (Vec<_>, Vec<_>) = deferred
                    .into_iter()
                    .partition(|m| m.thread_ts.as_deref() == Some(parent_ts));
                result.extend(matched);
                deferred = remaining;
            }
        }

        // Append any remaining deferred (parent not in this buffer = already ingested)
        result.extend(deferred);

        result
    }
}
```

- [ ] **Step 4: main.rs に module を追加**

```rust
pub mod buffer;
```

- [ ] **Step 5: テスト成功を確認**

Run: `cargo test --test buffer_test`
Expected: 3 tests PASS

- [ ] **Step 6: コミット**

```bash
git add src/buffer.rs tests/buffer_test.rs src/main.rs
git commit -m "feat: add sort buffer for message ordering guarantee"
```

---

## Task 6: Vegapunk gRPC クライアント (`vegapunk/client.rs`)

**Files:**
- Create: `src/vegapunk/mod.rs`
- Create: `src/vegapunk/client.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: vegapunk/mod.rs を作成**

```rust
// src/vegapunk/mod.rs
pub mod client;
pub use client::VegapunkClient;
```

- [ ] **Step 2: vegapunk/client.rs を実装**

```rust
// src/vegapunk/client.rs
use anyhow::{Context, Result};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tracing::{info, warn};

use crate::converter::IngestMessage;

// Generated from proto
pub mod proto {
    tonic::include_proto!("graphrag");
}

use proto::graph_rag_engine_client::GraphRagEngineClient;
use proto::{IngestRequest, IngestMessage as ProtoIngestMessage, MessageMetadata, ListSchemasRequest};

pub struct VegapunkClient {
    client: GraphRagEngineClient<tonic::service::interceptor::InterceptedService<Channel, AuthInterceptor>>,
}

#[derive(Clone)]
struct AuthInterceptor {
    token: String,
}

impl tonic::service::Interceptor for AuthInterceptor {
    fn call(&mut self, mut req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        let val: MetadataValue<_> = format!("Bearer {}", self.token)
            .parse()
            .map_err(|_| tonic::Status::internal("invalid auth token"))?;
        req.metadata_mut().insert("authorization", val);
        Ok(req)
    }
}

impl VegapunkClient {
    pub async fn connect(endpoint: &str, auth_token: &str) -> Result<Self> {
        let channel = Channel::from_shared(format!("http://{endpoint}"))
            .context("invalid gRPC endpoint")?
            .connect()
            .await
            .context("failed to connect to Vegapunk gRPC")?;

        let interceptor = AuthInterceptor {
            token: auth_token.to_string(),
        };

        let client = GraphRagEngineClient::with_interceptor(channel, interceptor);

        Ok(Self { client })
    }

    pub async fn check_schema_exists(&mut self, schema_name: &str) -> Result<bool> {
        let response = self
            .client
            .list_schemas(ListSchemasRequest {})
            .await
            .context("failed to list schemas")?;

        let exists = response
            .into_inner()
            .schemas
            .iter()
            .any(|s| s.name == schema_name);

        Ok(exists)
    }

    /// QueryNodes でチャンネルの最新 Message の timestamp を取得する（カーソル復元用）
    pub async fn get_latest_message_ts(
        &mut self,
        schema: &str,
        channel_id: &str,
    ) -> Result<Option<String>> {
        let request = proto::QueryNodesRequest {
            schema: schema.to_string(),
            node_type: "Message".to_string(),
            filters: vec![proto::AttributeFilter {
                key: "channel_id".to_string(),
                op: "eq".to_string(),
                value: channel_id.to_string(),
            }],
            sort_by: Some("timestamp".to_string()),
            sort_order: Some("desc".to_string()),
            limit: Some(1),
            offset: None,
            traverse: None,
        };

        let response = self
            .client
            .query_nodes(request)
            .await
            .context("failed to query nodes for cursor recovery")?;

        let nodes = response.into_inner().nodes;
        if let Some(node) = nodes.first() {
            let ts = node.attributes.iter()
                .find(|a| a.key == "timestamp")
                .map(|a| a.value.clone());
            Ok(ts)
        } else {
            Ok(None)
        }
    }

    pub async fn ingest(&mut self, messages: Vec<IngestMessage>, schema: &str) -> Result<i32> {
        let proto_messages: Vec<ProtoIngestMessage> = messages
            .into_iter()
            .map(|m| ProtoIngestMessage {
                id: Some(m.id),
                text: m.text,
                metadata: Some(MessageMetadata {
                    source_type: m.metadata.source_type,
                    author: m.metadata.author,
                    author_id: Some(m.metadata.author_id),
                    channel: m.metadata.channel,
                    channel_id: Some(m.metadata.channel_id),
                    thread_id: m.metadata.thread_id,
                    timestamp: m.metadata.timestamp,
                }),
            })
            .collect();

        let request = IngestRequest {
            messages: proto_messages,
            schema: schema.to_string(),
        };

        let response = self
            .client
            .ingest(request)
            .await
            .context("failed to ingest messages")?;

        Ok(response.into_inner().ingested_count)
    }
}
```

- [ ] **Step 3: main.rs に module を追加**

```rust
pub mod vegapunk;
```

- [ ] **Step 4: ビルド確認**

Run: `cargo build`
Expected: ビルド成功（proto の型と一致しているか確認。不一致があれば修正）

- [ ] **Step 5: コミット**

```bash
git add src/vegapunk/ src/main.rs
git commit -m "feat: add Vegapunk gRPC client with auth interceptor and ingest API"
```

---

## Task 7: CLI エントリポイント (`main.rs`)

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: main.rs を clap ベースの CLI に書き換え**

```rust
// src/main.rs
pub mod buffer;
pub mod config;
pub mod converter;
pub mod cursor;
pub mod vegapunk;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "vegapunk-slack-ingester")]
#[command(about = "Ingest Slack messages into Vegapunk")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the ingester daemon (catchup + realtime sync)
    Serve,
    /// Import historical messages for a channel
    Import {
        /// Slack channel ID to import
        #[arg(long)]
        channel: String,
        /// Start date for import (YYYY-MM-DD)
        #[arg(long)]
        since: String,
    },
    /// Health check
    Health,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve => {
            tracing::info!("starting serve mode");
            // TODO: implement in Task 9+
            todo!("serve mode not yet implemented")
        }
        Commands::Import { channel, since } => {
            tracing::info!(channel = %channel, since = %since, "starting import mode");
            // TODO: implement in Task 10
            todo!("import mode not yet implemented")
        }
        Commands::Health => {
            tracing::info!("health check");
            // TODO: implement in Task 11
            todo!("health check not yet implemented")
        }
    }
}
```

- [ ] **Step 2: ビルド確認**

Run: `cargo build`
Expected: ビルド成功

- [ ] **Step 3: CLI ヘルプを確認**

Run: `cargo run -- --help`
Expected: serve, import, health サブコマンドが表示される

- [ ] **Step 4: コミット**

```bash
git add src/main.rs
git commit -m "feat: add CLI with serve, import, and health subcommands"
```

---

## Task 8: Slack Web API クライアント (`slack/client.rs`)

**Files:**
- Create: `src/slack/mod.rs`
- Create: `src/slack/client.rs`
- Create: `src/slack/types.rs`
- Create: `src/cache.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: slack/types.rs を作成**

```rust
// src/slack/types.rs
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SlackApiResponse<T> {
    pub ok: bool,
    pub error: Option<String>,
    #[serde(flatten)]
    pub data: Option<T>,
}

#[derive(Debug, Deserialize)]
pub struct UserInfoData {
    pub user: SlackUser,
}

#[derive(Debug, Deserialize)]
pub struct SlackUser {
    pub id: String,
    pub real_name: Option<String>,
    pub profile: SlackUserProfile,
}

#[derive(Debug, Deserialize)]
pub struct SlackUserProfile {
    pub display_name: Option<String>,
    pub real_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConversationInfoData {
    pub channel: SlackChannel,
}

#[derive(Debug, Deserialize)]
pub struct SlackChannel {
    pub id: String,
    pub name: Option<String>,
    pub purpose: Option<SlackPurpose>,
}

#[derive(Debug, Deserialize)]
pub struct SlackPurpose {
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct HistoryData {
    pub messages: Vec<HistoryMessage>,
    pub has_more: Option<bool>,
    pub response_metadata: Option<ResponseMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct HistoryMessage {
    pub r#type: Option<String>,
    pub user: Option<String>,
    pub text: Option<String>,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub bot_id: Option<String>,
    pub reply_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseMetadata {
    pub next_cursor: Option<String>,
}
```

- [ ] **Step 2: cache.rs を作成**

```rust
// src/cache.rs
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct TtlCache {
    entries: HashMap<String, (String, Instant)>,
    ttl: Duration,
}

impl TtlCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: HashMap::new(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).and_then(|(value, inserted_at)| {
            if inserted_at.elapsed() < self.ttl {
                Some(value.as_str())
            } else {
                None
            }
        })
    }

    pub fn set(&mut self, key: String, value: String) {
        self.entries.insert(key, (value, Instant::now()));
    }
}
```

- [ ] **Step 3: slack/client.rs を作成**

```rust
// src/slack/client.rs
use anyhow::{bail, Context, Result};
use reqwest::Client;
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

use super::types::*;
use crate::cache::TtlCache;

pub struct SlackClient {
    http: Client,
    bot_token: String,
    user_cache: TtlCache,
    channel_cache: TtlCache,
}

impl SlackClient {
    pub fn new(bot_token: &str, cache_ttl_secs: u64) -> Self {
        Self {
            http: Client::new(),
            bot_token: bot_token.to_string(),
            user_cache: TtlCache::new(cache_ttl_secs),
            channel_cache: TtlCache::new(cache_ttl_secs),
        }
    }

    pub async fn get_user_name(&mut self, user_id: &str) -> Result<String> {
        if let Some(name) = self.user_cache.get(user_id) {
            return Ok(name.to_string());
        }

        let resp: SlackApiResponse<UserInfoData> = self
            .api_get("users.info", &[("user", user_id)])
            .await?;

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

        let resp: SlackApiResponse<HistoryData> = self.api_get("conversations.history", &params).await?;
        resp.data.context("missing history data")
    }

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
```

- [ ] **Step 4: slack/mod.rs を作成**

```rust
// src/slack/mod.rs
pub mod client;
pub mod types;
pub use client::SlackClient;
```

- [ ] **Step 5: main.rs に modules を追加**

```rust
pub mod cache;
pub mod slack;
```

- [ ] **Step 6: ビルド確認**

Run: `cargo build`
Expected: ビルド成功

- [ ] **Step 7: コミット**

```bash
git add src/slack/ src/cache.rs src/main.rs
git commit -m "feat: add Slack Web API client with rate limiting and caching"
```

---

## Task 9: serve モード実装 (`catchup.rs` + Socket Mode 統合)

**Files:**
- Create: `src/catchup.rs`
- Create: `src/slack/socket.rs`
- Modify: `src/main.rs`
- Modify: `src/slack/mod.rs`

- [ ] **Step 1: catchup.rs を実装**

```rust
// src/catchup.rs
use anyhow::Result;
use tracing::{info, warn};

use crate::config::Config;
use crate::converter::{slack_to_ingest, SlackMessage};
use crate::cursor::CursorStore;
use crate::slack::SlackClient;
use crate::vegapunk::VegapunkClient;

pub async fn run_catchup(
    config: &Config,
    slack: &mut SlackClient,
    vegapunk: &mut VegapunkClient,
    cursor_store: &CursorStore,
) -> Result<()> {
    let cursors = cursor_store.load()?;
    let channels = &config.slack_watch_channel_ids;

    if channels.is_empty() {
        info!("no watch channels configured, skipping catchup");
        return Ok(());
    }

    for channel_id in channels {
        // Cursor recovery: local file first, then QueryNodes fallback
        let oldest = if let Some(ts) = cursors.get(channel_id) {
            info!(channel_id, cursor = %ts, "catching up from local cursor");
            ts.clone()
        } else {
            // Fallback: query Vegapunk for latest message in this channel
            match vegapunk.get_latest_message_ts("slack-ingester", channel_id).await {
                Ok(Some(ts)) => {
                    info!(channel_id, cursor = %ts, "catching up from Vegapunk QueryNodes");
                    ts
                }
                Ok(None) => {
                    info!(channel_id, "no data in Vegapunk, skipping catchup (use import for initial load)");
                    continue;
                }
                Err(e) => {
                    warn!(channel_id, error = %e, "failed to query Vegapunk for cursor, skipping");
                    continue;
                }
            }
        };

        let channel_name = slack
            .get_channel_name(channel_id)
            .await
            .unwrap_or_else(|_| channel_id.clone());

        let mut cursor_page: Option<String> = None;
        let mut last_ts: Option<String> = None;

        loop {
            let history = slack
                .conversations_history(
                    channel_id,
                    Some(&oldest),
                    cursor_page.as_deref(),
                    200,
                )
                .await?;

            let mut batch: Vec<SlackMessage> = Vec::new();

            for msg in &history.messages {
                // Skip bot messages
                if msg.bot_id.is_some() {
                    continue;
                }
                let Some(ref user_id) = msg.user else {
                    continue;
                };
                let Some(ref text) = msg.text else {
                    continue;
                };

                let user_name = slack.get_user_name(user_id).await.unwrap_or_else(|_| user_id.clone());

                batch.push(SlackMessage {
                    text: text.clone(),
                    user_id: user_id.clone(),
                    user_name,
                    channel_id: channel_id.clone(),
                    channel_name: channel_name.clone(),
                    ts: msg.ts.clone(),
                    thread_ts: msg.thread_ts.clone(),
                });

                // Fetch thread replies if this is a thread parent
                if msg.reply_count.unwrap_or(0) > 0 && msg.thread_ts.is_none() {
                    let replies = slack.conversations_replies(channel_id, &msg.ts).await?;
                    for reply in &replies.messages {
                        if reply.ts == msg.ts {
                            continue; // skip parent (already added)
                        }
                        if reply.bot_id.is_some() {
                            continue;
                        }
                        let Some(ref reply_user) = reply.user else {
                            continue;
                        };
                        let Some(ref reply_text) = reply.text else {
                            continue;
                        };
                        let reply_user_name = slack
                            .get_user_name(reply_user)
                            .await
                            .unwrap_or_else(|_| reply_user.clone());

                        batch.push(SlackMessage {
                            text: reply_text.clone(),
                            user_id: reply_user.clone(),
                            user_name: reply_user_name,
                            channel_id: channel_id.clone(),
                            channel_name: channel_name.clone(),
                            ts: reply.ts.clone(),
                            thread_ts: reply.thread_ts.clone(),
                        });
                    }
                }

                last_ts = Some(msg.ts.clone());
            }

            // Sort by ts ascending and ingest in batches
            batch.sort_by(|a, b| a.ts.cmp(&b.ts));

            for chunk in batch.chunks(config.ingest_batch_size) {
                let ingest_msgs: Vec<_> = chunk
                    .iter()
                    .map(|m| slack_to_ingest(m, "slack-ingester"))
                    .collect();
                let count = vegapunk.ingest(ingest_msgs, "slack-ingester").await?;
                info!(channel_id, ingested = count, "batch ingested");
            }

            // Update cursor after each page
            if let Some(ref ts) = last_ts {
                cursor_store.save_channel(channel_id, ts)?;
            }

            // Pagination
            let has_more = history.has_more.unwrap_or(false);
            if !has_more {
                break;
            }
            cursor_page = history
                .response_metadata
                .and_then(|m| m.next_cursor)
                .filter(|c| !c.is_empty());
            if cursor_page.is_none() {
                break;
            }

            // Rate limiting: wait between pages
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        }

        info!(channel_id, "catchup complete");
    }

    Ok(())
}
```

- [ ] **Step 2: slack/socket.rs のスケルトンを作成**

```rust
// src/slack/socket.rs
use anyhow::Result;
use tracing::{info, warn, error};

use crate::buffer::SortBuffer;
use crate::config::Config;
use crate::converter::{slack_to_ingest, SlackMessage};
use crate::cursor::CursorStore;
use crate::slack::SlackClient;
use crate::vegapunk::VegapunkClient;

/// Run Socket Mode event loop.
/// This function blocks until the process is shut down.
pub async fn run_socket_mode(
    config: &Config,
    slack: &mut SlackClient,
    vegapunk: &mut VegapunkClient,
    cursor_store: &CursorStore,
) -> Result<()> {
    // Socket Mode の実装は slack-morphism クレートの API に依存するため、
    // 具体的なコードは slack-morphism のドキュメントを参照して実装する。
    //
    // 処理フロー:
    // 1. SLACK_APP_TOKEN で Socket Mode WebSocket 接続を確立
    // 2. message イベントを受信
    // 3. Bot 自身の発言をフィルタ (bot_id チェック)
    // 4. Sort Buffer に投入
    // 5. ウィンドウ (config.sort_buffer_window_secs) 経過後に flush
    // 6. flush されたメッセージを ingest API に投入
    // 7. カーソルを更新
    //
    // slack-morphism の SocketModeClient を使う場合:
    //   let socket_client = SlackClientSocketModeListener::new(...);
    //   socket_client.listen(event_handler).await;

    info!("socket mode not yet fully implemented — waiting for slack-morphism integration");
    // Placeholder: keep the process alive
    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

- [ ] **Step 3: slack/mod.rs を更新**

```rust
pub mod client;
pub mod socket;
pub mod types;
pub use client::SlackClient;
```

- [ ] **Step 4: main.rs の serve コマンドを実装**

```rust
Commands::Serve => {
    let config = config::Config::from_env()?;

    // Connect to Vegapunk
    let mut vegapunk = vegapunk::VegapunkClient::connect(
        &config.vegapunk_grpc_endpoint,
        &config.vegapunk_auth_token,
    )
    .await?;
    tracing::info!("connected to Vegapunk");

    // Check schema exists
    if !vegapunk.check_schema_exists("slack-ingester").await? {
        anyhow::bail!("schema 'slack-ingester' does not exist. Create it via admin UI before starting the ingester.");
    }
    tracing::info!("schema 'slack-ingester' confirmed");

    // Initialize Slack client
    let mut slack_client = slack::SlackClient::new(&config.slack_bot_token, 3600);

    // Cursor store
    let cursor_store = cursor::CursorStore::new(&config.cursor_file_path);

    // Catchup
    catchup::run_catchup(&config, &mut slack_client, &mut vegapunk, &cursor_store).await?;

    // Socket Mode
    slack::socket::run_socket_mode(&config, &mut slack_client, &mut vegapunk, &cursor_store).await?;

    Ok(())
}
```

main.rs の先頭に追加:
```rust
pub mod catchup;
```

- [ ] **Step 5: ビルド確認**

Run: `cargo build`
Expected: ビルド成功

- [ ] **Step 6: コミット**

```bash
git add src/catchup.rs src/slack/socket.rs src/slack/mod.rs src/main.rs
git commit -m "feat: implement serve mode with catchup and socket mode skeleton"
```

---

## Task 10: import サブコマンド (`import.rs`)

**Files:**
- Create: `src/import.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: import.rs を実装**

```rust
// src/import.rs
use anyhow::{Context, Result};
use chrono::NaiveDate;
use tracing::info;

use crate::config::Config;
use crate::converter::{slack_to_ingest, SlackMessage};
use crate::slack::SlackClient;
use crate::vegapunk::VegapunkClient;

pub async fn run_import(
    config: &Config,
    slack: &mut SlackClient,
    vegapunk: &mut VegapunkClient,
    channel_id: &str,
    since: &str,
) -> Result<()> {
    // Parse since date to unix timestamp
    let since_date = NaiveDate::parse_from_str(since, "%Y-%m-%d")
        .with_context(|| format!("invalid date format: {since}. Expected YYYY-MM-DD"))?;
    let since_ts = since_date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let oldest = format!("{since_ts}.000000");

    let channel_name = slack
        .get_channel_name(channel_id)
        .await
        .unwrap_or_else(|_| channel_id.to_string());

    info!(channel_id, channel_name, since, "starting import");

    let mut total_count: u32 = 0;
    let mut cursor_page: Option<String> = None;

    loop {
        let history = slack
            .conversations_history(channel_id, Some(&oldest), cursor_page.as_deref(), 200)
            .await?;

        let mut batch: Vec<SlackMessage> = Vec::new();

        for msg in &history.messages {
            if msg.bot_id.is_some() {
                continue;
            }
            let Some(ref user_id) = msg.user else {
                continue;
            };
            let Some(ref text) = msg.text else {
                continue;
            };

            let user_name = slack.get_user_name(user_id).await.unwrap_or_else(|_| user_id.clone());

            batch.push(SlackMessage {
                text: text.clone(),
                user_id: user_id.clone(),
                user_name,
                channel_id: channel_id.to_string(),
                channel_name: channel_name.clone(),
                ts: msg.ts.clone(),
                thread_ts: msg.thread_ts.clone(),
            });

            // Fetch thread replies
            if msg.reply_count.unwrap_or(0) > 0 && msg.thread_ts.is_none() {
                let replies = slack.conversations_replies(channel_id, &msg.ts).await?;
                for reply in &replies.messages {
                    if reply.ts == msg.ts {
                        continue;
                    }
                    if reply.bot_id.is_some() {
                        continue;
                    }
                    let Some(ref reply_user) = reply.user else {
                        continue;
                    };
                    let Some(ref reply_text) = reply.text else {
                        continue;
                    };
                    let reply_user_name = slack
                        .get_user_name(reply_user)
                        .await
                        .unwrap_or_else(|_| reply_user.clone());

                    batch.push(SlackMessage {
                        text: reply_text.clone(),
                        user_id: reply_user.clone(),
                        user_name: reply_user_name,
                        channel_id: channel_id.to_string(),
                        channel_name: channel_name.clone(),
                        ts: reply.ts.clone(),
                        thread_ts: reply.thread_ts.clone(),
                    });
                }

                // Rate limit for replies API
                tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            }
        }

        // Sort and ingest
        batch.sort_by(|a, b| a.ts.cmp(&b.ts));

        for chunk in batch.chunks(config.ingest_batch_size) {
            let ingest_msgs: Vec<_> = chunk
                .iter()
                .map(|m| slack_to_ingest(m, "slack-ingester"))
                .collect();
            let count = vegapunk.ingest(ingest_msgs, "slack-ingester").await?;
            total_count += count as u32;
        }

        // Pagination
        let has_more = history.has_more.unwrap_or(false);
        if !has_more {
            break;
        }
        cursor_page = history
            .response_metadata
            .and_then(|m| m.next_cursor)
            .filter(|c| !c.is_empty());
        if cursor_page.is_none() {
            break;
        }

        // Rate limit for history API
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    }

    info!(channel_id, total_count, "import complete");
    Ok(())
}
```

- [ ] **Step 2: main.rs の import コマンドを実装**

```rust
Commands::Import { channel, since } => {
    let config = config::Config::from_env()?;

    let mut vegapunk = vegapunk::VegapunkClient::connect(
        &config.vegapunk_grpc_endpoint,
        &config.vegapunk_auth_token,
    )
    .await?;

    if !vegapunk.check_schema_exists("slack-ingester").await? {
        anyhow::bail!("schema 'slack-ingester' does not exist.");
    }

    let mut slack_client = slack::SlackClient::new(&config.slack_bot_token, 3600);

    import::run_import(&config, &mut slack_client, &mut vegapunk, &channel, &since).await?;

    Ok(())
}
```

main.rs の先頭に追加:
```rust
pub mod import;
```

- [ ] **Step 3: ビルド確認**

Run: `cargo build`
Expected: ビルド成功

- [ ] **Step 4: コミット**

```bash
git add src/import.rs src/main.rs
git commit -m "feat: implement import subcommand for historical message ingestion"
```

---

## Task 11: Dockerfile + docker-compose.yml

**Files:**
- Create: `Dockerfile`
- Create: `docker-compose.yml`

- [ ] **Step 1: Dockerfile を作成**

```dockerfile
# Build stage
FROM rust:1.82-slim AS builder

RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs rust-toolchain.toml ./
COPY proto/ proto/
COPY src/ src/

RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false ingester
USER ingester

COPY --from=builder /app/target/release/vegapunk-slack-ingester /app/vegapunk-slack-ingester

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD /app/vegapunk-slack-ingester health

ENTRYPOINT ["/app/vegapunk-slack-ingester"]
CMD ["serve"]
```

- [ ] **Step 2: docker-compose.yml を作成**

```yaml
services:
  slack-ingester:
    build: .
    restart: unless-stopped
    env_file: .env
    volumes:
      - cursor-data:/app/data

volumes:
  cursor-data:
```

- [ ] **Step 3: Docker ビルド確認**

Run: `docker compose build`
Expected: ビルド成功

- [ ] **Step 4: コミット**

```bash
git add Dockerfile docker-compose.yml
git commit -m "feat: add Dockerfile and docker-compose for container deployment"
```

---

## Task 12: 障害通知 (`alert.rs`)

**Files:**
- Create: `src/alert.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: alert.rs を実装**

```rust
// src/alert.rs
use anyhow::Result;
use reqwest::Client;
use tracing::{error, info};

pub struct AlertClient {
    http: Client,
    bot_token: String,
    channel_id: Option<String>,
}

impl AlertClient {
    pub fn new(bot_token: &str, channel_id: Option<String>) -> Self {
        Self {
            http: Client::new(),
            bot_token: bot_token.to_string(),
            channel_id,
        }
    }

    pub async fn send(&self, message: &str) -> Result<()> {
        let Some(ref channel_id) = self.channel_id else {
            return Ok(()); // No alert channel configured
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
```

- [ ] **Step 2: main.rs に module を追加**

```rust
pub mod alert;
```

- [ ] **Step 3: ビルド確認**

Run: `cargo build`
Expected: ビルド成功

- [ ] **Step 4: コミット**

```bash
git add src/alert.rs src/main.rs
git commit -m "feat: add alert client for failure notifications"
```

---

## 依存関係

```
Task 1 (proto + 初期化)
  └─ Task 2 (config)
       ├─ Task 3 (converter)
       ├─ Task 4 (cursor)
       ├─ Task 5 (buffer)
       ├─ Task 6 (vegapunk client)
       └─ Task 7 (CLI)
            ├─ Task 8 (slack client)
            │    ├─ Task 9 (serve = catchup + socket)
            │    └─ Task 10 (import)
            ├─ Task 11 (Docker)
            └─ Task 12 (alert)
```

Tasks 3, 4, 5, 6 は互いに独立しており並行実装可能。
