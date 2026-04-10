# vegapunk-slack-ingester 要件定義

Slack のメッセージを Vegapunk に ingest するツールの要件。設計は含まない。

---

## 1. ゴール

Slack ワークスペースのメッセージを、継続的に Vegapunk の `discussion` スキーマに従って Graph として蓄積する。ingester が止まっていた期間のメッセージも漏れなく蓄積する。

---

## 2. スコープ

### 2.1 やること

- Slack のメッセージを受信する
- Slack のメッセージを `discussion` スキーマのノード・エッジに変換する
- Vegapunk に投入する
- 起動時に、前回停止中のメッセージをキャッチアップして投入する

### 2.2 やらないこと

- LLM 呼び出し(判断を伴わない)
- Slack への投稿(障害通知を除く)
- メッセージ内容に対する解釈・要約・分類

---

## 3. リポジトリ

- `SIVIRA/vegapunk-slack-ingester`（GitHub、プライベート）
- Vegapunk 本体（`SIVIRA/vegapunk`）とは完全に独立したリポジトリ
- Vegapunk 本体への変更は原則行わない

---

## 4. 技術的制約

- **言語**: Rust（常駐プロセスのため。TypeScript は常駐には使わない、Python は使わない）
- **実行形態**: Docker コンテナ
- **稼働ホスト**: Vegapunk 本体が動いているホスト
- **Vegapunk との通信**: gRPC（Vegapunk 本体が gRPC サーバ）

---

## 5. 観察対象

以下の 2 種類を観察対象とする。

1. **Bot が参加している Slack チャンネル/DM**
2. **明示的に監視対象として指定されたチャンネル**（Bot 不参加でも ingest する）

Bot 自身の発言は ingest しない。

---

## 6. Slack から取得すべき情報

各メッセージについて以下を取得する。

- メッセージ本文
- 発言者（Slack user ID、display name または real name）
- 発言チャンネル/DM（Slack channel ID、チャンネル名、purpose）
- タイムスタンプ（`message_ts`）
- スレッド親の ts（スレッド返信の場合）
- 本文中の @mention 対象の Slack user ID

---

## 7. Vegapunk に作るべき Graph 構造

### 7.1 対象スキーマ

Vegapunk の `discussion` スキーマ v5（完成形を §7.2 に示す）。

v5 は現行 v4 に対して以下の変更を含む。これらの変更は Vegapunk 本体リポジトリ側での適用が必要であり、Ryugo さんの承認事項。

- `Channel` ノードに `external_id` 属性を追加（Slack channel ID を一意に識別するため）

### 7.2 スキーマ v5 の完成形

```yaml
name: discussion
version: 5
min_compatible: 1
description: チャット・議事録・意思決定・プロジェクト管理
nodes:
  Thread:
    attributes:
      created_at:
        type: string
        required: true
      summary:
        type: string
        required: false
  File:
    attributes:
      extracted_text:
        type: string
        required: false
      url:
        type: string
        required: true
      file_type:
        type: string
        required: false
  Task:
    attributes:
      name:
        type: string
        required: true
      assignee:
        type: string
        required: false
      status:
        type: string
        required: false
  Person:
    attributes:
      name:
        type: string
        required: true
      external_id:
        type: string
        required: false
  Message:
    attributes:
      timestamp:
        type: string
        required: true
      text:
        type: string
        required: true
      source_type:
        type: string
        required: true
      external_id:
        type: string
        required: false
  Alternative:
    attributes:
      rejection_reason:
        type: string
        required: false
      summary:
        type: string
        required: true
  Topic:
    attributes:
      name:
        type: string
        required: true
      category:
        type: string
        required: false
  CommunitySummary:
    attributes:
      level:
        type: int
        required: true
      summary:
        type: string
        required: true
      model:
        type: string
        required: false
      created_at:
        type: string
        required: true
      generation:
        type: int
        required: true
      prompt_hash:
        type: string
        required: false
  Rationale:
    attributes:
      summary:
        type: string
        required: true
  Decision:
    attributes:
      decided_at:
        type: string
        required: false
      status:
        type: string
        required: false
      summary:
        type: string
        required: true
  Meeting:
    attributes:
      date:
        type: string
        required: false
      title:
        type: string
        required: true
  Project:
    attributes:
      name:
        type: string
        required: true
      status:
        type: string
        required: false
      description:
        type: string
        required: false
  Specification:
    attributes:
      status:
        type: string
        required: false
      version:
        type: string
        required: false
      summary:
        type: string
        required: true
  Phase:
    attributes:
      name:
        type: string
        required: true
      order:
        type: int
        required: false
  Community:
    attributes:
      member_count:
        type: int
        required: true
      created_at:
        type: string
        required: true
      level:
        type: int
        required: true
      generation:
        type: int
        required: true
  Channel:
    attributes:
      source_type:
        type: string
        required: false
      name:
        type: string
        required: true
      purpose:
        type: string
        required: false
      external_id:
        type: string
        required: false
edges:
  BECAUSE:
    from: Decision
    to: Rationale
  SUPERSEDES:
    from: Specification
    to: Specification
  SAID:
    from: Person
    to: Message
  DEFINES:
    from: Decision
    to: Specification
  ABOUT:
    from: Thread
    to:
    - Project
    - Task
  IN_THREAD:
    from: Message
    to: Thread
  RELATED_TO:
    from: Thread
    to: Thread
  DEPENDS_ON:
    from: Task
    to: Task
  HAS_SUMMARY:
    from: Community
    to: CommunitySummary
  IN_CHANNEL:
    from:
    - Thread
    - Message
    to: Channel
  ATTENDED:
    from: Meeting
    to: Person
  LED_TO:
    from: Thread
    to: Decision
  REFERENCES:
    from: Message
    to: File
  REJECTED:
    from: Decision
    to: Alternative
  BELONGS_TO:
    from: '*'
    to: Community
  LINKED_TO:
    from: Task
    to:
    - Decision
    - Specification
  PRODUCED:
    from: Meeting
    to: Thread
  HAS_TOPIC:
    from: '*'
    to: Topic
  HAS_TASK:
    from: Phase
    to: Task
  HAS_PHASE:
    from: Project
    to: Phase
  REPLIES_TO:
    from: Message
    to: Message
  MENTIONS:
    from: Message
    to: Person
traceable_pairs:
- claim: Decision
  evidence: Rationale
  edge: BECAUSE
```

### 7.3 ingester が作るノード（スキーマ上のノード型のうち、このツールが作るもの）

- `Person`（Slack ユーザー）
- `Channel`（Slack チャンネル/DM）
- `Message`（Slack メッセージ）

上記以外のノード型（`Thread`, `Decision`, `Topic`, `Task` 等）はこのツールでは作らない。

### 7.4 ingester が作るエッジ

- `Person --SAID--> Message`（発言者）
- `Message --IN_CHANNEL--> Channel`（所属チャンネル）
- `Message --REPLIES_TO--> Message`（スレッド返信の親）
- `Message --MENTIONS--> Person`（@mention の対象）

上記以外のエッジはこのツールでは作らない。

### 7.5 一意性の保証

- `Person.external_id` = Slack user ID
- `Channel.external_id` = Slack channel ID
- `Message.external_id` = Slack `message_ts`

同じ `external_id` を持つノードは Vegapunk 側で一意に識別される（重複投入しない）。

---

## 8. 時系列と順序の保証

以下の順序が構造上保証されなければならない。

- `REPLIES_TO` エッジを張る時点で、参照先の親メッセージ（Message ノード）が Vegapunk 側に既に存在すること
- 言い換えると、メッセージは常に時系列順に親 → 子の順で投入される

この保証のため、ingester 内でのメッセージ処理は **単一ワーカによる時系列直列処理** とする。並行処理しない。

---

## 9. キャッチアップ（取りこぼし防止）

- ingester が停止していた期間のメッセージも、起動時に全て投入されること
- キャッチアップの起点は「前回どこまで投入したか」の記録
- キャッチアップでも時系列順の直列処理を守る
- Slack API のレート制限に対応する

---

## 10. 冪等性

- 同じメッセージを二度実行しても、Vegapunk 側に重複が生まれないこと
- ingester の再起動、キャッチアップとリアルタイム受信の切り替わり、手動再実行、いずれでも重複が発生しないこと

---

## 11. 死活監視

- **コンテナレベル**: プロセスが生きているかどうかを Docker が監視し、落ちたら自動再起動する
- **プロセスレベル**: 稼働中は自分の責務（取りこぼしのない ingest）を全うする

致命的なエラー時に Slack へ通知する仕組みは、要件としては必須ではないが、あった方が望ましい。通知先は Ryugo さんが指定する。

---

## 12. セキュリティ

- Slack トークン、Vegapunk 認証情報など一切の認証情報をハードコードしない
- 環境変数経由で注入する
- `.env.example` にはプレースホルダーのみ記載
- ログに認証情報・機密情報を出力しない

SIVIRA 共通の `security-rules.md` に従う。

---

## 13. コーディング規約

SIVIRA 共通の `coding-standards.md` に従う。

---

## 14. Progressive Disclosure

`CLAUDE.md` には常時必要な情報のみ記載し、詳細は `specs/` 配下に分離する。

---

## 15. レビュー

実装が一段落した時点で、`reviewer-agent.md` の手順に従って ryugo-agent-01 の reviewer エージェントにコードレビューを依頼する。

---

## 16. 実装前に Ryugo さんと合意が必要な事項

1. Vegapunk `discussion` スキーマを v5 に更新し、`Channel.external_id` を追加すること
2. Vegapunk の gRPC エンドポイントとクライアント API の確認（`SIVIRA/vegapunk` の `docs/specs/client-guide.md` を参照）
3. Vegapunk の認証方式（JWT 等）と ingester 用トークンの発行
4. Slack アプリの作成と OAuth スコープの確定
5. 障害通知の送信先（Slack チャンネル）
