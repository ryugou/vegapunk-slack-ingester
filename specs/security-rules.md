# セキュリティルール

SIVIRA 共通規約（`~/.claude/specs/security-rules.md`）に加え、本プロジェクト固有のルールを定める。

---

## 認証情報の管理

- Slack Bot Token（`SLACK_BOT_TOKEN`）、App Token（`SLACK_APP_TOKEN`）、Vegapunk 認証情報はすべて環境変数で注入する。
- `.env` ファイルは `.gitignore` に含め、Git 管理下に入れない。
- `.env.example` にはプレースホルダー（`xoxb-your-bot-token-here` 等）のみ記載する。

---

## ログへの機密情報出力禁止

- Slack トークン、Vegapunk 認証情報をログに出力しない。
- メッセージ本文のログ出力は `debug` レベルに限定し、本番運用では `info` 以上に設定する。
- ユーザーの個人情報（real name、email 等）をログに含める場合は必要最小限とする。

---

## Slack API トークンの権限

- Bot Token のスコープは必要最小限にする（要件 §16 で確定予定）。
- App-Level Token は Socket Mode 接続に必要な `connections:write` のみ。

---

## gRPC 通信

- Vegapunk との通信は同一ホスト内のコンテナ間通信を前提とする。
- 認証方式が JWT 等で確定した場合、トークンは環境変数で注入する。

---

## Docker セキュリティ

- コンテナは非 root ユーザーで実行する。
- 不要なパッケージをイメージに含めない（マルチステージビルドで最小化）。
