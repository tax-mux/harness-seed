# コンテキスト内の用途別マッピング

LLM に渡す **1 回の Chat Completions リクエスト**（HarnessSeed では `TurnPromptContext::render`）を、**用途・記憶の層・格納先**で整理した図解。

- ReAct 実装: [react-implementation.md](react-implementation.md)
- 最少行動単位: [agent-minimum-action-unit.md](agent-minimum-action-unit.md)
- 組み込みツール: [builtin_tools/README.md](builtin_tools/README.md)

**現状（2025-05 時点）**

| 区分 | 状態 |
|------|------|
| ターン内 `TurnTrace` | 実装済み |
| セッション短期 `SessionMemory` → `Previous turns` | **実装済み**（§10） |
| コンテキスト計測・`logs/context.jsonl` | 実装済み |
| 外部記憶（検索・diary 等） | 未接続（`PromptBlocks::recalled` の差し込み口のみ） |
| ルールファイル注入（`prompt.rules_paths`） | **実装済み**（`PromptBlocks`） |
| trace / session の要約 | 未実装 |

---

## 1. 一枚絵：コンテキストウィンドウの中身

```mermaid
flowchart TB
    subgraph ctx["1 回の LLM 呼び出しに載るコンテキスト"]
        direction TB
        SYS["system ブロック"]
        USR["user ブロック（複数セクション）"]
    end

    subgraph sys_parts["system の用途"]
        R["振る舞いルール・出力形式"]
        T["ツール定義・カタログ"]
        L["長期参照（任意）rules / SSoT 抜粋"]
    end

    subgraph usr_parts["user の用途"]
        M["中期・長期の想起（任意）外部 search"]
        P["短期：Previous turns ✓"]
        G["今回のゴール User input"]
        TR["短期：Turn trace so far"]
        NX["指示 Next step JSON 等"]
    end

    SYS --> R
    SYS --> T
    SYS --> L
    USR --> M
    USR --> P
    USR --> G
    USR --> TR
    USR --> NX

    sys_parts -.-> SYS
    usr_parts -.-> USR
```

**原則**

| 層 | 主な載せ方 | 変わる頻度 |
|----|------------|------------|
| 不変に近い方針 | **system** | 低 |
| 今回の依頼・途中経過 | **user**（ブロック分割） | 高 |

---

## 2. 記憶の層 × 格納先

```mermaid
flowchart LR
    subgraph short["短期"]
        ST1["TurnTrace\n（ターン内）"]
        ST2["SessionMemory\n（REPL セッション）"]
        ST3["プロンプト上の\nPrevious turns / trace"]
    end

    subgraph mid["中期"]
        MD1["外部ストア\ndiary / search"]
    end

    subgraph long["長期"]
        LG1["正本ドキュメント\nrules / SSoT"]
        LG2["外部 KG\n/ drawer"]
    end

    subgraph proc["HarnessSeed プロセス内"]
        ST1
        ST2
    end

    subgraph ext["外部ストア"]
        MD1
        LG1
        LG2
    end

    MD1 -.->|ターン開始時に検索→user| ST3
    LG1 -.->|起動時 rules 読込→system or user| ST3
    ST1 --> ST3
    ST2 --> ST3
```

| 記憶の層 | 役割 | 格納先（推奨） | HarnessSeed 現状 |
|----------|------|----------------|----------------|
| **短期** | いまのターン・直近の発話 | ヒープ `TurnTrace` / `SessionMemory` → **user ブロック** | trace + **SessionMemory**（`Previous turns`、REPL `clear` でリセット） |
| **中期** | 数日〜数週の作業・経緯 | 外部 diary, search | 未接続 |
| **長期** | 規約・正本・関係 | 正本ドキュメント / 外部 KG | 未接続 |

---

## 3. user ブロック内の順序（推奨レイアウト）

上から下へ **安定 → 依頼 → 最新の事実** の順が読みやすい。

```mermaid
block-beta
    columns 1

    block:recall:2
        columns 1
        recallTitle["Recalled context（任意）"]
        memSearch["外部 search 結果"]
        docRecall["正本・rules 抜粋"]
    end

    block:session:2
        columns 1
        sessTitle["Session memory（任意）"]
        prev["Previous turns: 直近 N ターン ✓"]
    end

    block:task:1
        columns 1
        goal["User input: 今回のゴール"]
    end

    block:work:2
        columns 1
        traceTitle["Working memory（ターン内）"]
        trace["Turn trace so far\nthought / action / observation"]
    end

    block:cue:1
        columns 1
        next["Next step JSON:"]
```

| 順序 | セクション | 用途 | 典型ソース |
|------|------------|------|------------|
| 1 | Recalled context | 長期・中期からの引用 | 外部ストア、rules ファイル |
| 2 | Previous turns | 短期（会話の続き） | `SessionMemory` |
| 3 | User input | **今回のタスク** | REPL 1 行入力 |
| 4 | Turn trace so far | **作業中の事実** | `TurnTrace` |
| 5 | Next step JSON | 出力形式のリマインド | ハーネス固定文言 |

HarnessSeed **現行**（`src/llm/brain.rs`）は **2 + 3 + 4 + 5**（`Previous turns` あり。1・外部記憶 / rules 注入は未接続）。

---

## 4. system ブロック内の用途

```mermaid
flowchart TB
    SYS["ChatMessage::system"]

    SYS --> A["ReAct ルール\nJSON 1 オブジェクトのみ"]
    SYS --> B["スキーマ\nthought / action / answer"]
    SYS --> C["Available tools 一覧"]
    SYS --> D["Tool catalog\n（tools_catalog 動的）"]
    SYS --> E["長期ポリシー（任意）\nrules / SSoT 抜粋"]

    style E stroke-dasharray: 5 5
```

| 内容 | 載せ方 | 更新頻度 |
|------|--------|----------|
| 出力形式・禁止事項 | system 固定 | コード変更時 |
| ツール名・引数の要約 | system + catalog | ツール追加時 |
| プロジェクト憲法・rules | system 先頭または末尾（任意） | 正本ドキュメント更新時 |

**今回のゴールは system に入れない**（user の `User input` へ）。

---

## 5. ライフサイクル：いつ何が載るか

```mermaid
sequenceDiagram
    participant U as ユーザー
    participant H as HarnessSeed
    participant ST as 短期メモリ
    participant MP as 外部中期記憶
    participant DOC as 正本ドキュメント
    participant M as LLM

    Note over H,DOC: 起動時（任意）
    H->>DOC: rules / SSoT 読込
    DOC-->>H: 長期ブロック

    U->>H: REPL 入力（1ターン開始）
    H->>MP: search（任意）
    MP-->>H: 中期ブロック
    H->>ST: SessionMemory から Previous turns 読込

    loop 各 decide（1ターン内）
        H->>M: system + user（Previous turns + trace）
        M-->>H: thought / action / answer
        H->>ST: TurnTrace に追記
    end

    H->>ST: SessionMemory.push_turn（user + answer）
    H->>MP: diary_write（任意）
    H-->>U: Answer
```

| タイミング | 短期 | 中期 | 長期 |
|------------|------|------|------|
| プロセス起動 | — | — | rules / 正本の読み込み（任意・未） |
| ターン開始 | **実装**: `SessionMemory` → `Previous turns` を user 先頭へ | search → user（未） | 読み込み（未） |
| 各 `decide` | `TurnTrace` 増殖（ターン内のみ再送） | — | — |
| ターン終了 | **実装**: `push_turn(user, answer)` | diary_write（未） | 通常は書かない |
| REPL `clear` | **実装**: `SessionMemory::clear()` | — | — |

---

## 6. コンテキストが長いとき（忘却）

```mermaid
flowchart LR
    subgraph window["コンテキストウィンドウ（上限あり）"]
        direction LR
        S["system"]
        OLD["古い user / trace"]
        NEW["新しい trace"]
        OUT["生成余白"]
    end

    S --- OLD --- NEW --- OUT

    OLD -.->|溢れたら落ちる| X["実質忘却"]
    NEW --> M["モデルが参照しやすい"]

    MP["外部ストアへ退避"] -.->|ターン終了前| OLD
```

| 現象 | 原因 | 対策 |
|------|------|------|
| ターン内で古い observation が効かない | trace が線形に増える | trace 要約・最後の N ステップのみ |
| REPL 前の発話を忘れる（K 超過） | `session_max_turns` を超えた古いターンは捨てられる | K の調整、`clear`、将来は外部ストアへ退避 |
| 溢れた内容の完全喪失 | ウィンドウ外 | 外部 diary / ローカル要約ファイルへ退避 |

計測は `[context step]` / `logs/context.jsonl` の `prompt_tokens` で確認。

---

## 7. 用途 × 載せ方 早見表

| 用途 | 載せ方 | 記憶層 | 推奨ストア |
|------|--------|--------|------------|
| JSON 出力形式のみ | system | — | コード |
| ツール一覧 | system + catalog | — | コード + [builtin_tools](builtin_tools/README.md) |
| プロジェクト rules / 遂行規約 | system（抜粋） | 長期 | rules ファイル / 正本 |
| プロジェクト仕様・決定 | user 先頭 or system | 長期 | 正本ドキュメント |
| 似た過去作業の想起 | user 先頭 | 中期 | 外部 search |
| 直近の会話 | user `Previous turns` | 短期 | プロセス内 SessionMemory |
| 今回の依頼 | user `User input` | 短期 | 入力そのもの |
| Thought / Action / Observation | user `Turn trace` | 短期 | TurnTrace |
| ツール実行結果の原文 | trace 内 observation | 短期 | TurnTrace |
| セッションまたぎの日記 | （注入 or ツール） | 中期 | 外部 diary |
| エンティティ関係 | user 抜粋 or ツール | 長期 | 外部 KG |

---

## 8. HarnessSeed 現行との対応

```mermaid
flowchart TB
    subgraph impl["実装済み"]
        I1["SYSTEM_PROMPT + Tool catalog → system"]
        I2["Previous turns + User input + Turn trace → user"]
        I3["TurnTrace / context_usages"]
        I4["SessionMemory（REPL セッション）"]
        I5["logs/context.jsonl"]
    end

    subgraph planned["推奨・未実装"]
        P1["外部記憶ブリッジ"]
        P2["trace / session 要約"]
    end

    I1 --> SYS["system"]
    I2 --> USR["user"]
    I3 --> USR
    I4 --> I2
    I5 -.->|計測ログ。推論入力ではない| LOG["ファイル"]
```

| マッピング上の要素 | ソース |
|--------------------|--------|
| system ルール・ツール | `src/llm/brain.rs` `SYSTEM_PROMPT` + `tools_catalog()` |
| user `Previous turns` | `src/session.rs` `format_for_prompt` ← `ReActLoop.session` |
| user ゴール + trace | `src/llm/brain.rs` `build_messages` |
| 短期ターン内 | `src/action.rs` `TurnTrace` |
| 短期セッション跨ぎ | `src/session.rs` `SessionMemory` / `src/react.rs` |
| 計測 | `src/context_metrics.rs`, `src/context_log.rs` |

---

## 10. 短期記憶（SessionMemory）実装

REPL セッション中だけ有効な **完了ターンの履歴** を保持し、次ターン以降の LLM 呼び出しに `Previous turns:` として載せる。

### 10.1 データ構造

```mermaid
flowchart LR
    RL["ReActLoop"]
    SM["SessionMemory"]
    PT["PastTurn × N"]
    RL -->|所有| SM
    SM --> PT
    PT --> U["user_input"]
    PT --> A["answer（最終応答のみ）"]
```

| 型 | ファイル | 内容 |
|----|----------|------|
| `PastTurn` | `src/session.rs` | 1 ターン分の `user_input` + `answer` |
| `SessionMemory` | `src/session.rs` | `Vec<PastTurn>` + 上限設定 |

**意図的に保存しないもの**

- ターン内の `thought` / `action` / `observation`（`TurnTrace` はターン終了で破棄）
- ツール出力の全文（要約が必要なら将来 §10.4）

### 10.2 プロンプトへの載せ方

`LlmBrain::build_messages`（`src/llm/brain.rs`）の user ブロック構成:

```
{Previous turns:（空なら省略）}

User input:
{今回の REPL 1 行}

Turn trace so far:
{当ターンの trace}

Next step JSON:
```

`Previous turns` の書式（`SessionMemory::format_for_prompt`）:

```text
Previous turns:
[turn 1]
User: ...
Assistant: ...
[turn 2]
...
```

### 10.3 ライフサイクル（実装）

```mermaid
sequenceDiagram
    participant R as ReActLoop
    participant S as SessionMemory
    participant B as AgentBrain
    participant L as LLM

    Note over R,S: ターン開始時点の S は「前までに完了したターン」のみ

    loop max_steps
        R->>B: decide(TurnPromptContext)
        B->>L: messages（blocks + S → Previous turns）
        L-->>B: thought / action / answer
        R->>R: TurnTrace 更新
    end

    R->>S: push_turn(input, answer)
    Note over S: 超過分は先頭から remove(0)
```

| 操作 | タイミング | コード |
|------|------------|--------|
| 読み取り | 各 `decide` | `TurnPromptContext { blocks, input, trace, session }` |
| 書き込み | `Answer` 確定直後 | `session.push_turn(user_input, answer)` |
| リセット | REPL `clear` / `forget` / `reset` | `session.clear()`（`src/react.rs` `run_repl`） |

`AgentBrain::decide` の第 3 引数 `session` は **全頭脳で受け取る**が、現状プロンプトへ反映するのは **LlmBrain のみ**（`SimpleRuleBrain` は未使用）。

### 10.4 上限と設定

| 項目 | 既定 | 設定 |
|------|------|------|
| 保持ターン数 | 8 | `react.session_max_turns`（`config/config.json`） |
| 1 フィールド最大文字 | 2000 文字 | コード定数 `SessionMemory::DEFAULT_MAX_CHARS_PER_FIELD`（超過は `…` で切り詰め） |

`ReActConfig::session_max_turns` ← `AppConfig::react_config()` で解決。`ReActLoop::new` 時に `SessionMemory::new(...)` を生成。

### 10.5 観測・確認

- **REPL**: 2 ターン目以降、同じセッションで「さっき言ったこと」を聞く
- **ログ**: `logs/context.jsonl` の各 step の `prompt` に `Previous turns` が含まれる（ターン 2 以降）
- **stderr**: `[context step]` の `prompt_tokens` がターン進行とともに増える傾向

### 10.6 未実装（短期の次の一手）

| 項目 | 説明 |
|------|------|
| trace 要約 | 1 ターン内ステップが多いときのトークン抑制 |
| session 要約 | N ターン超の「意味」を圧縮して Previous turns を短くする |
| 永続化 | プロセス終了で `SessionMemory` は消える（ファイル / 外部ストア連携は未） |
| ルール頭脳 | `SimpleRuleBrain` が session を参照した応答（デモ用・任意） |

---

## 11. 関連ドキュメント

| ドキュメント | 内容 |
|--------------|------|
| [react-implementation.md](react-implementation.md) | ReAct ループ・頭脳・ログ |
| [agent-minimum-action-unit.md](agent-minimum-action-unit.md) | Action = 1 Tool Call |
| [builtin_tools/README.md](builtin_tools/README.md) | ツール仕様 |
| [../config/README.md](../config/README.md) | 実行時設定（`session_max_turns` 含む） |
