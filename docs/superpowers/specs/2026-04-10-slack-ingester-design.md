# vegapunk-slack-ingester 設計ドキュメント

## 1. 概要

Slack ワークスペースのメッセージを Vegapunk の `ingest` API に継続的に投入する常駐プロセス。
Slack メッセージを適切な metadata 付きで投入し、Graph 構造の構築は Vegapunk の ingest ハンドラおよび LLM パイプラインに委ねる。

### 1.1 要件からの変更点

以下は要件定義（`specs/requirements.md`）からブレスト・レビューを経て変更・明確化された事項。

| 要件箇所 | 変更内容 | 理由 |
|---|---|---|
| §7.1 スキーマ v5 | 変更不要（既に適用済み） | discussion スキーマは v5 に更新済み |
| §7.3〜§7.4 ノード・エッジ構築 | ingester が直接作るのではなく、`ingest` API に metadata を付けて投入する | Vegapunk の公開 API（`ingest`）を使い、Graph 構築を委ねる |
| スキーマ | `discussion` ではなく `slack-ingester` を新規作成して使用 | 調整・再 Ingest が頻発する初期段階で `slack` 名を温存するため。安定後に `slack` への移行を検討 |
| §16 合意事項 1 | 除外 | スキーマ v5 更新は完了済み |

---

## 2. アーキテクチャ

### 2.1 実行モード

同一バイナリで 2 つのサブコマンドを提供する。

| サブコマンド | 説明 |
|---|---|
| `serve` | 常駐プロセス。起動時に差分キャッチアップを行い、以降は Socket Mode でリアルタイム同期する |
| `import` | 一回実行。指定チャンネル・期間の過去メッセージを Vegapunk に投入する |

`serve` と `import` は並行実行できる。両者とも Vegapunk の `ingest` API を叩くだけで ingester 側に共有状態がなく、冪等性は `id` フィールドで保証されるため重複は発生しない。

**Slack API レート制限の注意:** レート制限はワークスペース × アプリ単位で共有される（§5.6 参照）。`serve` と `import` は同じ Slack App のトークンを使うため、制限枠を共有する。ただし `serve` のリアルタイム受信（Socket Mode）は Web API を呼ばないため、レート制限に影響しない。`import` 実行中に `serve` のキャッチアップが走らない限り、実用上問題になることはない。多重に `import` を実行する場合は注意が必要。

### 2.2 serve モードの構成

```
┌─────────────┐     Socket Mode      ┌──────────────────────┐
│   Slack API  │◄────(WebSocket)─────►│  vegapunk-slack-     │
│              │                      │  ingester serve      │
│              │     Web API          │                      │
│              │◄────(HTTP)──────────►│  ┌────────────────┐  │
└─────────────┘     (キャッチアップ)    │  │ Slack Receiver │  │
                                      │  └───────┬────────┘  │
                                      │          │           │
                                      │  ┌───────▼────────┐  │
                                      │  │ Sort Buffer    │  │
                                      │  │ (in-memory)    │  │
                                      │  └───────┬────────┘  │
                                      │          │           │
                                      │  ┌───────▼────────┐  │
                                      │  │ Ingester       │  │
                                      │  │ (単一ワーカ)    │  │
                                      │  └───────┬────────┘  │
                                      └──────────┼───────────┘
                                                 │ gRPC (ingest)
                                      ┌──────────▼───────────┐
                                      │  Vegapunk            │
                                      │  (host.docker.       │
                                      │   internal:6840)     │
                                      └──────────────────────┘
```

### 2.3 コンポーネント

| コンポーネント | 責務 |
|---|---|
| **Slack Receiver** | Socket Mode で Slack からリアルタイムにメッセージイベントを受信する。キャッチアップ時は Web API（`conversations.history` / `conversations.replies`）でメッセージを取得する |
| **Sort Buffer** | 受信したメッセージを `ts` でソートしてから下流に渡すバッファ。Socket Mode のイベント到着順は厳密に時系列順でないため、順序保証（§7）のために必要 |
| **Ingester** | バッファからメッセージを取り出し、`ingest` API の形式に変換して Vegapunk に投入する単一ワーカ |

### 2.4 serve モードの処理フロー

```
1. 起動
   ├─ Vegapunk に接続確認（gRPC health check）
   ├─ slack-ingester スキーマの存在確認（なければ作成）
   ├─ キャッチアップ処理（§5）
   └─ Socket Mode 接続開始

2. リアルタイム受信（定常状態）
   ├─ Socket Mode でメッセージイベントを受信
   ├─ Bot 自身の発言をフィルタ
   ├─ 監視対象チャンネルかを判定
   ├─ Sort Buffer に投入
   ├─ バッファウィンドウ経過後、ts 順にソートして flush
   ├─ ingest 形式に変換
   └─ Vegapunk ingest API に投入

3. シャットダウン
   ├─ Socket Mode 切断
   ├─ バッファ内の未処理メッセージを flush・投入完了
   └─ プロセス終了
```

---

## 3. Slack 連携

### 3.1 接続方式

**Socket Mode** を使用する。

- パブリック URL が不要で、ファイアウォール内の vegapunk ホストでそのまま動作する
- `SLACK_APP_TOKEN`（`xapp-`）で WebSocket 接続を確立する
- `SLACK_BOT_TOKEN`（`xoxb-`）で Web API を呼び出す

### 3.2 必要な Slack App の設定

| 項目 | 設定 |
|---|---|
| Socket Mode | 有効 |
| Event Subscriptions | `message.channels`, `message.groups`, `message.im`, `message.mpim` |
| Bot Token Scopes | `channels:history`, `channels:read`, `groups:history`, `groups:read`, `im:history`, `im:read`, `mpim:history`, `mpim:read`, `users:read` |
| App-Level Token Scopes | `connections:write` |

### 3.3 観察対象の判定

メッセージイベントを受信した際、以下のいずれかに該当すれば ingest 対象とする:

1. Bot が参加しているチャンネル/DM（Socket Mode で自動的にイベントが届く）
2. `SLACK_WATCH_CHANNEL_IDS` に指定されたチャンネル（Bot 不参加でも `conversations.history` で取得）

Bot 自身の発言（`bot_id` が自分自身）は除外する。

### 3.4 取得する情報

Socket Mode の message イベントペイロードには `user`（ID のみ）、`channel`（ID のみ）、`text`、`ts` しか含まれない。display name やチャンネル名は含まれないため、`users.info` と `conversations.info` API の追加呼び出しが必要（確認済み）。

| 情報 | 取得元 | `ingest` metadata へのマッピング |
|---|---|---|
| メッセージ本文 | `event.text` | `text` |
| 発言者 Slack user ID | `event.user` | `metadata.author_id` |
| 発言者の表示名 | `users.info` API（追加呼び出し） | `metadata.author` |
| チャンネル ID | `event.channel` | `metadata.channel_id` |
| チャンネル名 | `conversations.info` API（追加呼び出し） | `metadata.channel` |
| タイムスタンプ | `event.ts` | `metadata.timestamp`（RFC3339 に変換） |
| スレッド親 ts | `event.thread_ts` | `metadata.thread_id` |
| @mention 対象 | `event.text` 内の `<@U...>` | `text` に含まれる（Vegapunk 側で抽出） |

### 3.5 ユーザー情報・チャンネル情報のキャッシュ

`users.info` と `conversations.info` の呼び出しを毎メッセージ行うと Slack API のレート制限に抵触する。以下のキャッシュ戦略を採る:

- **インメモリキャッシュ**（TTL は設定可能、デフォルト 1 時間）で user ID → display name、channel ID → channel name をキャッシュ
- キャッシュミス時のみ API を呼び出す
- Person ノードの同定は `external_id`（Slack user ID）ベースで行われるため、表示名が一時的にキャッシュ上で古くても同定の正確性には影響しない
- プロセス再起動時はキャッシュが空になるが、Slack API のレート制限の範囲内で問題なく再構築できる

---

## 4. Vegapunk 連携

### 4.1 接続情報

| 項目 | 値 |
|---|---|
| gRPC エンドポイント | `VEGAPUNK_GRPC_ENDPOINT`（デフォルト: `host.docker.internal:6840`） |
| 認証 | Bearer token（`VEGAPUNK_AUTH_TOKEN`） |
| スキーマ名 | `slack-ingester` |

### 4.2 スキーマの前提

`slack-ingester` スキーマは事前に作成済みであることを前提とする（admin UI または CLI で手動作成）。起動時にスキーマの存在を確認し、存在しない場合はエラー終了する。

```
ListSchemas → slack-ingester が存在するか確認
  ├─ 存在する → 起動続行
  └─ 存在しない → エラーログを出力してプロセス終了
```

**スキーマ定義:** `slack-ingester` スキーマは `discussion` テンプレート（v5）をそのまま使用する。完全な定義は `specs/requirements.md` §7.2 を参照。ingester が投入するメッセージから構築されるノード・エッジは §4.5 に記載。Discussion テンプレートに含まれる他のノード型（Decision, Topic, Task 等）は ingester では使用しないが、Vegapunk の LLM extraction パイプラインがメッセージ内容から抽出する可能性がある。

### 4.3 ingest API への投入形式

```json
{
  "messages": [
    {
      "id": "C06ABC123-1712345678.123456",
      "text": "デプロイの手順を確認したいんですが、@田中さん 前回のリリースノートありますか？",
      "metadata": {
        "source_type": "slack",
        "author": "山田太郎",
        "author_id": "U01ABC123",
        "channel": "#engineering",
        "channel_id": "C06ABC123",
        "thread_id": "1712345600.000001",
        "timestamp": "2026-04-10T14:30:00+09:00"
      }
    }
  ],
  "schema": "slack-ingester"
}
```

**フィールドの設計意図:**

| フィールド | 値の形式 | 意図 |
|---|---|---|
| `id` | `{channel_id}-{message_ts}` | メッセージの一意識別。channel_id を含めることで公式仕様上の一意性を保証し、チャンネル単位のカーソル管理やデバッグ時の可読性にも寄与する |
| `source_type` | `"slack"` | 固定値。他のソース（議事録等）と区別する |
| `author` | display name または real name | ingest ハンドラが Person ノードの `name` として直接使用する |
| `author_id` | Slack user ID | ingest ハンドラが Person ノードの `external_id` として直接使用する |
| `channel` | チャンネル名（`#` 付き） | ingest ハンドラが Channel ノードの `name` として直接使用する |
| `channel_id` | Slack channel ID | ingest ハンドラが Channel ノードの `external_id` として直接使用する |
| `thread_id` | 親メッセージの `ts` | スレッド構造の復元に使われる |
| `timestamp` | RFC3339 | Vegapunk の要件。Slack の `ts`（Unix epoch）から変換 |

### 4.4 バッチ投入

- リアルタイム受信時: メッセージを 1 件ずつ即時投入する（低レイテンシ優先）
- キャッチアップ / import 時: バッチ投入する（デフォルト 20 件、`INGEST_BATCH_SIZE` 環境変数で設定可能）

### 4.5 Vegapunk 側で構築される Graph 構造

ingester は metadata を付けて `ingest` API に投入するのみ。Graph 構造は Vegapunk 側で構築される。構築は 2 段階で行われる:

**ingest ハンドラが metadata から直接構築するもの（LLM 不要、確実）:**

- **Message ノード**: `text`, `timestamp`, `source_type`, `id` から生成
- **Person ノード**: `author` → `name`, `author_id` → `external_id`
- **Channel ノード**: `channel` → `name`, `channel_id` → `external_id`
- **Thread ノード**: `thread_id` から生成
- **SAID エッジ**: Person → Message
- **IN_THREAD エッジ**: Message → Thread
- **IN_CHANNEL エッジ**: Thread → Channel

**LLM extraction パイプラインが text から抽出するもの（品質は LLM 依存）:**

- Decision / Rationale / Alternative / Specification / Topic 等の高レベルエンティティ
- LED_TO / BECAUSE / REJECTED 等の高レベルエッジ

**確認済み（2026-04-11）:** 上記の ingest ハンドラの動作は Vegapunk 本体側で確認済み。

**ingester の方針:** ingester は ingest API のみを使う。Low-level API（UpsertEdges 等）は使わない。投入後のデータを直接検証し、Graph 構造が不十分な場合は Vegapunk 側の ingest API をブラッシュアップする。ingest API の改善で対応できない場合は、新たな公開 API として Vegapunk に追加することを検討する。

**現時点で ingest ハンドラが生成しないエッジ:**

- REPLIES_TO（Message → Message）、MENTIONS（Message → Person）は現行の ingest ハンドラでは自動生成されない
- ingester は `thread_id` や `<@U...>` を含む metadata を投入するので、Vegapunk 側で対応されれば機能する
- これは ingester の課題ではなく、Vegapunk の ingest API の品質改善サイクルで対応する

---

## 5. キャッチアップ（取りこぼし防止）

### 5.1 serve と import の責務分離

| 機能 | serve | import |
|---|---|---|
| 初回のチャンネル登録時 | 何もしない（カーソルが存在しないチャンネルはスキップ） | `import --channel C123 --since 2026-01-01` で過去メッセージを投入 |
| 二回目以降の起動 | 前回停止後の差分をキャッチアップ | 不要（serve が差分を拾う） |
| 新規チャンネル追加時 | 追加後のリアルタイム受信のみ | `import` で過去分を投入。serve 稼働中でも実行可能 |

### 5.2 カーソルの復元

ローカルカーソル（JSON ファイル、Docker volume にマウント）で管理する。

ingest 成功時にチャンネルごとの最終 `message_ts` をローカルファイルに書き込む。

```json
{
  "C06ABC123": "1712345678.123456",
  "C07DEF456": "1712345700.654321"
}
```

| 状況 | 動作 |
|---|---|
| ローカルカーソルあり | ローカル値を使う（高速、ネットワーク不要） |
| ローカルカーソルなし + Vegapunk にデータあり | QueryNodes で復元 |
| ローカルカーソルなし + Vegapunk にもデータなし | キャッチアップスキップ（`import` の責務） |

**Vegapunk QueryNodes API（フォールバック）:**

ローカルカーソルにエントリがないチャンネルについては、Vegapunk の `QueryNodes` API（PR #35, main マージ済み）で最新の Message の timestamp を取得する。

```
QueryNodes(
  schema: "slack-ingester",
  node_type: "Message",
  filters: [{key: "channel_id", op: "eq", value: "<channel_id>"}],
  sort_by: "timestamp",
  sort_order: "desc",
  limit: 1
)
```

レスポンスの `nodes[0].attributes["timestamp"]` がそのチャンネルの最終投入時刻。

### 5.3 キャッチアップの処理フロー（serve 起動時）

```
1. 監視対象チャンネルの一覧を取得
2. ローカルカーソルファイルを読み込み
3. カーソルが存在するチャンネルについて:
   a. カーソル以降のメッセージを conversations.history で取得
   b. スレッドがあれば conversations.replies で返信を取得
   c. 時系列順にソート（親メッセージ → 子メッセージ）
   d. バッチ単位で ingest API に投入
   e. ローカルカーソルを更新
4. 全チャンネルのキャッチアップ完了後、Socket Mode に移行
```

### 5.4 import サブコマンドの処理フロー

```
vegapunk-slack-ingester import --channel C06ABC123 --since 2026-01-01
```

```
1. 指定チャンネルの情報を取得（conversations.info）
2. --since 以降のメッセージを conversations.history で取得
3. スレッドがあれば conversations.replies で返信を取得
4. 時系列順にソート（親メッセージ → 子メッセージ）
5. バッチ単位で ingest API に投入
6. 完了後、処理件数を出力してプロセス終了
```

### 5.5 キャッチアップと Socket Mode の切り替わり

キャッチアップ完了後に Socket Mode を開始する。キャッチアップ中に発生したメッセージは、Socket Mode 開始後に受信される。

**重複防止:** `ingest` API の `id` フィールド（`{channel_id}-{message_ts}`）により、同じメッセージが二度投入されても Vegapunk 側でデータ整合性は保たれる（upsert）。ただし、同一メッセージの再投入時には extraction ジョブが再実行される可能性があり、LLM 推論コストが発生する。ingester 側のカーソル管理（§5.2）が一次防衛線であり、Vegapunk 側の upsert は二次防衛線という位置づけ。

### 5.6 Slack API レート制限への対応

レート制限は **ワークスペース × アプリ単位** で、API メソッドごとに適用される。

| API | Tier | レート制限 | 対応 |
|---|---|---|---|
| `conversations.history` | Tier 3 | 50+ req/min | リクエスト間に 1.2 秒以上の間隔 |
| `conversations.replies` | Tier 3 | 50+ req/min | 同上 |
| `users.info` | Tier 4 | 100+ req/min | キャッシュ + リクエスト間 0.6 秒以上 |
| `conversations.info` | Tier 3 | 50+ req/min | キャッシュ + リクエスト間 1.2 秒以上 |

**バースト制限:** Slack は具体的なバースト制限値を非公開としている。推奨は 1 req/sec。

**HTTP 429 への対応:** `Retry-After` ヘッダー（秒数）に従ってリトライする。

**serve と import の並行実行時の影響:**

- `serve` のリアルタイム受信は Socket Mode（WebSocket）であり、Web API のレート制限枠を消費しない
- `serve` のキャッチアップと `import` が同時に走ると、同じ Tier のメソッド（`conversations.history` 等）のレート制限枠を共有する
- `import` を同時に複数実行すると枠を圧迫する可能性がある。実装では 429 レスポンスに適切に対応する（`Retry-After` に従う）が、**`import` の多重実行は避けること**

---

## 6. 冪等性

冪等性は以下の 2 層で保証する:

**一次防衛線（ingester 側のカーソル管理）:**

キャッチアップ時に、ローカルカーソルまたは QueryNodes で復元した最終投入時刻以降のメッセージのみを取得する。これにより、通常運用では同じメッセージを二度投入することを防ぐ。

**二次防衛線（Vegapunk 側の upsert）:**

`ingest` API の `id` フィールド（`{channel_id}-{message_ts}`）で一意識別する。Vegapunk のグラフ・ベクトルストレージは upsert 方式であり、同一ノードの再投入はデータを上書きするだけで整合性は保たれる。

ただし、ジョブが completed 状態になった後に同じ `id` で再 ingest すると、extraction ジョブが再実行される（partial unique index は completed を対象外としているため）。データ整合性は壊れないが、**LLM 推論コストが無駄に発生する**。一次防衛線（カーソル管理）で可能な限り再投入を防ぐことが重要。

---

## 7. 時系列と順序の保証

### 7.1 単一ワーカによる直列処理

メッセージの処理は単一の非同期タスクが時系列順に逐次実行する。並行処理は行わない。

### 7.2 順序の保証方法

要件 §8 では「`REPLIES_TO` エッジを張る時点で、参照先の親メッセージが Vegapunk 側に既に存在すること」を構造上の保証として求めている。以下の仕組みでこれを保証する。

**キャッチアップ / import 時:**

- `conversations.history` で `oldest` パラメータ指定により時系列昇順で取得する
- 各メッセージについてスレッドがある場合、`conversations.replies` で返信を取得する
- 親メッセージを投入した後に、その返信を投入する

**リアルタイム受信時:**

Socket Mode のイベント到着順は厳密に時系列順でない場合がある。以下の仕組みで順序を保証する:

1. **Sort Buffer:** 受信したイベントを即座に投入せず、一定時間（デフォルト 5 秒、`SORT_BUFFER_WINDOW_SECS` 環境変数で設定可能）バッファリングする
2. **ts ソート:** ウィンドウ経過後、バッファ内のメッセージを `ts` の昇順にソートする
3. **親メッセージ優先:** スレッド返信（`thread_ts` が存在するメッセージ）は、同一バッファ内に親メッセージがあれば親を先に投入する。親がバッファにない場合は既に投入済みと判断する
4. **flush:** ソート済みのメッセージを順次 `ingest` API に投入する

---

## 8. エラーハンドリング

### 8.1 Vegapunk 接続エラー

| エラー | 対応 |
|---|---|
| `UNAVAILABLE` | Exponential backoff でリトライ（初回 1 秒、最大 60 秒） |
| `UNAUTHENTICATED` | ログ出力してプロセス終了（トークンの問題は人間が対処） |
| `INVALID_ARGUMENT` | ログ出力してそのメッセージをスキップ（データ変換の問題） |

### 8.2 Slack 接続エラー

| エラー | 対応 |
|---|---|
| Socket Mode 切断 | 自動再接続（Slack SDK の組み込み機能） |
| HTTP 429（レート制限） | `Retry-After` ヘッダーに従ってリトライ |
| HTTP 5xx | Exponential backoff でリトライ |

### 8.3 致命的エラー時の通知

`SLACK_ALERT_CHANNEL_ID` が設定されている場合、以下のイベントで Slack に通知する:

- Vegapunk への接続が一定時間以上復旧しない
- 連続して ingest が失敗する
- プロセスが異常終了する

通知の閾値（接続断の時間、連続失敗の件数等）は環境変数で設定可能とする。デフォルト値は Ryugo さんと合意の上で決定する。

---

## 9. 死活監視

### 9.1 Docker ヘルスチェック

```dockerfile
HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD /app/vegapunk-slack-ingester health
```

`health` サブコマンドは以下を確認する:
- プロセスが応答すること
- Slack の Socket Mode 接続が確立されていること
- 直近の ingest が一定時間以内に成功していること

### 9.2 コンテナ再起動

Docker の `restart: unless-stopped` ポリシーにより、プロセスが落ちた場合は自動で再起動する。再起動時はキャッチアップ処理が走り、停止中のメッセージを投入する。

---

## 10. 設定

### 10.1 環境変数

| 変数名 | 必須 | 説明 |
|---|---|---|
| `SLACK_BOT_TOKEN` | Yes | Slack Bot Token（`xoxb-`） |
| `SLACK_APP_TOKEN` | Yes | Slack App-Level Token（`xapp-`） |
| `SLACK_WATCH_CHANNEL_IDS` | No | 明示的に監視するチャンネル ID（カンマ区切り） |
| `VEGAPUNK_GRPC_ENDPOINT` | No | Vegapunk gRPC エンドポイント（デフォルト: `host.docker.internal:6840`） |
| `VEGAPUNK_AUTH_TOKEN` | Yes | Vegapunk Bearer token |
| `SLACK_ALERT_CHANNEL_ID` | No | 障害通知先の Slack チャンネル ID |
| `INGEST_BATCH_SIZE` | No | キャッチアップ / import 時のバッチサイズ（デフォルト: 20） |
| `SORT_BUFFER_WINDOW_SECS` | No | Sort Buffer のウィンドウ秒数（デフォルト: 5） |
| `RUST_LOG` | No | ログレベル（デフォルト: `vegapunk_slack_ingester=info`） |

### 10.2 Docker Compose

```yaml
services:
  slack-ingester:
    build: .
    restart: unless-stopped
    env_file: .env
    volumes:
      - cursor-data:/app/data  # ローカルカーソルの永続化
    # macOS Docker では network_mode: host は使えない。
    # VEGAPUNK_GRPC_ENDPOINT=http://host.docker.internal:6840 で接続する。
    # Linux では network_mode: host に変更し、localhost:6840 で直接接続可能。

volumes:
  cursor-data:
```

---

## 11. プロジェクト構成

```
vegapunk-slack-ingester/
├── CLAUDE.md
├── Cargo.toml
├── Cargo.lock
├── Dockerfile
├── docker-compose.yml
├── .env.example
├── .gitignore
├── .claudeignore
├── specs/
│   ├── requirements.md
│   ├── coding-standards.md
│   └── security-rules.md
├── docs/
│   └── superpowers/
│       └── specs/
│           └── 2026-04-10-slack-ingester-design.md  ← 本ドキュメント
└── src/
    ├── main.rs              # エントリポイント、CLI（serve / import / health）
    ├── config.rs            # 環境変数の読み込み・バリデーション
    ├── slack/
    │   ├── mod.rs
    │   ├── client.rs        # Slack Web API クライアント
    │   ├── socket.rs        # Socket Mode 接続・イベント受信
    │   └── types.rs         # Slack API の型定義
    ├── vegapunk/
    │   ├── mod.rs
    │   └── client.rs        # Vegapunk gRPC クライアント（ingest / ListSchemas）
    ├── ingester.rs          # メッセージ変換 + 単一ワーカの投入ループ
    ├── catchup.rs           # キャッチアップ処理（serve 起動時の差分同期）
    ├── import.rs            # import サブコマンドの処理
    ├── cursor.rs            # カーソル管理（ローカルファイル + QueryNodes フォールバック）
    ├── buffer.rs            # Sort Buffer（リアルタイム受信時の順序保証バッファ）
    └── cache.rs             # ユーザー/チャンネル情報のインメモリキャッシュ
```

---

## 12. 主要な依存クレート

| クレート | 用途 |
|---|---|
| `tokio` | 非同期ランタイム |
| `tonic` | gRPC クライアント（Vegapunk 接続） |
| `reqwest` | HTTP クライアント（Slack Web API） |
| `tokio-tungstenite` | WebSocket（Slack Socket Mode） |
| `serde` / `serde_json` | JSON シリアライズ/デシリアライズ |
| `chrono` | タイムスタンプ変換（Unix epoch → RFC3339） |
| `tracing` / `tracing-subscriber` | 構造化ログ |
| `anyhow` / `thiserror` | エラーハンドリング |
| `clap` | CLI 引数パーサー |

---

## 13. Vegapunk 側の前提・依存事項

以下は ingester の実装に先立ち、**Vegapunk 本体の改修が必要な事項**。「合意事項」ではなく、ingester が要件を満たすために Vegapunk 側のコード変更が前提となる項目。

| # | 事項 | 状態 |
|---|---|---|
| 1 | `QueryNodes` API（属性ベース構造化クエリ） | **実装済み（PR #35, main マージ済み）**。初期リリースからカーソル復元に使用 |
| 2 | REPLIES_TO / MENTIONS エッジの自動生成 | 現行 ingest ハンドラでは未生成。ingester は metadata を投入済みなので、Vegapunk 側の ingest API 改善で対応予定。ingester のブロッカーではない |

---

## 14. 実装前に合意が必要な事項

1. **proto ファイルの取得:** `sivira/vegapunk-proto` を git submodule として管理する（admin と同じ方式）。vegapunk ホストの deploy key に `sivira/vegapunk-proto` への read access を付与する必要あり
2. Slack アプリの作成と OAuth スコープの確定
3. 障害通知の送信先（Slack チャンネル）
4. 障害通知の閾値（接続断の時間、連続失敗の件数等）
