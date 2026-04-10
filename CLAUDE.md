# vegapunk-slack-ingester

Slack メッセージを Vegapunk の `discussion` スキーマ（v5）に従って Graph として継続的に蓄積する常駐プロセス。

## 技術スタック

- **言語:** Rust（stable）
- **実行形態:** Docker コンテナ（Vegapunk と同一ホスト）
- **Vegapunk 通信:** gRPC
- **Slack 通信:** Socket Mode（リアルタイム）+ Web API（キャッチアップ）

## 核となるルール

- **直列処理:** メッセージは単一ワーカで時系列順に処理する。並行処理しない
- **冪等性:** 同じメッセージの二重投入で Vegapunk 側に重複を生まない
- **キャッチアップ:** 起動時に前回停止中のメッセージを漏れなく投入する
- **Bot 自身の発言は ingest しない**

## ビルド・テスト・Lint

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

すべてコンテナ内で実行する。ホスト直接実行は避ける。

## 詳細仕様

- 要件定義: `specs/requirements.md`
- コーディング規約: `specs/coding-standards.md`
- セキュリティ: `specs/security-rules.md`
