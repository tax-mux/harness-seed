# 設定ファイル

## レイアウト

| パス | 役割 |
|------|------|
| `config/config.json.sample` | **既定のひな形**（リポジトリに固定。秘密情報なし） |
| `config/config.json` | **実行時に読む正本**（gitignore。ローカルで編集する） |
| `config/samples/config.*.json` | コネクタ別のひな形（リポジトリに固定） |

初回セットアップ:

```bash
cp config/config.json.sample config/config.json
# 必要なら llm.model / api_key などを編集
```

## プロバイダの切り替え

使いたいサンプルを `config.json` にコピーして上書きします。

```bash
# Ollama
cp config/samples/config.ollama.json config/config.json

# LM Studio
cp config/samples/config.lmstudio.json config/config.json

# OpenAI（API キーは環境変数 OPENAI_API_KEY でも可）
cp config/samples/config.openai.json config/config.json

# Google Gemini（API キーは環境変数 GEMINI_API_KEY）
cp config/samples/config.gemini.json config/config.json
export GEMINI_API_KEY=your-key

# Anthropic Claude（Messages API 直）
cp config/samples/config.anthropic.json config/config.json
export ANTHROPIC_API_KEY=your-key
```

別パスを直接指定する場合:

```bash
cargo run -- --config config/samples/config.lmstudio.json
```

環境変数 `HARNESS_SEED_CONFIG`（旧 `MYHARNESS_CONFIG`）でもパスを指定できます。

## プロジェクト資産（CLI: `config.agent.json`）

CLI 起動時、**実行時 cwd** に `config.agent.json` があれば自動読み込みします。

```bash
# プロジェクトルートで（config.agent.json を自動検出）
cargo run --release

# 明示指定
cargo run --release -- --agent-dir .agent
cargo run --release -- --config-agent ./config.agent.json
```

`config.agent.json` 例:

```json
{
  "workspace": ".",
  "agent_dir": ".agent"
}
```

| `agent_dir` 配下 | 内容 |
|------------------|------|
| `rules/**/*.md` | 追加ルール（再帰） |
| `skills/<id>/task.json` | 計画層タスク（スキル） |
| `skills/<id>/SKILL.md` | スキル説明（ルールへ注入） |
| `tools/*.json` | 宣言的シェルツール |

`workspace` は `HARNESS_WORKSPACE` に設定され、`list_dir` / `run_cmd` 等の基準になります。

## `memory` セクション（外部記憶ブリッジ）

ターン開始時に `recalled` へ直近作業・検索ヒットを注入し、ターン終了時に diary へ要約を書きます。

**local（プロセス内 diary）は外部で置き換えません。** セッション作業は常に local に残り、mempalace などは `backends` で**追加**します。不通時も local だけは動きます。

| キー | 意味 | 既定 |
|------|------|------|
| `local` | プロセス内 diary を使う | `backends` 指定時は `true` |
| `backends` | 追加バックエンド名の配列 | `[]` |
| `providers` | 名前 → 固有設定（本体は解釈しない） | `{}` |
| `recent_work.*` / `search.*` | チャネル別 retrieve オプション | 下記 |
| `rag.*` | アダプタ手前の記憶 RAG（作業ログ / 知識の分岐） | 下記 |
| `recall_max_rounds` | 計画層が `{"step":"recall"}` で追加検索できる回数（0=無効） | `2` |

ターン開始時は **記憶 RAG**（`src/memory/rag.rs`）が先に分岐する。アダプタ（local / mempalace）は `recent_work` / `search` の I/O だけ。

| 経路 | いつ | Bridge |
|------|------|--------|
| 作業ログ | `work_log=true`（続き系） | `recent_work` |
| 知識検索 | `knowledge=true` | `search(queries[])` |

| レイヤ | 役割 |
|--------|------|
| `local` | 今セッションの作業（常設推奨） |
| `mempalace` 等 | 長期・横断検索（追加、feature `mempalace`） |

共通オプション:

| キー | 意味 | 既定 |
|------|------|------|
| `recent_work.enabled` | 作業ログ経路で直近 diary を取る | `true` |
| `recent_work.max_entries` | レイヤあたりの件数 | `3` |
| `recent_work.max_chars` | `[recent work]` 文字上限 | `800` |
| `search.enabled` | 知識経路で検索する | `true` |
| `search.top_k` | 検索ヒット上限 | `5` |
| `search.max_chars` | `[search hit]` 文字上限 | `3200` |
| `rag.router` | `rule`（ヒント語）\| `llm`（JSON 分岐、失敗時 rule） | `llm` |
| `rag.max_queries` | 知識検索語の上限 | `3` |

`providers.mempalace`（Cursor の `mcp.json` と同じ起動が既定）:

| キー | 意味 | 既定 |
|------|------|------|
| `protocol` | `mcp_stdio` \| `tools_path` \| `mcp_jsonrpc` | `mcp_stdio` |
| `command` | MCP 実行ファイル | `python`（Windows は `C:\Python312\python.exe` があればそれ） |
| `args` | MCP 引数 | `["-m","mempalace.mcp_server"]` |
| `agent_name` | エージェント room 名（diary の書き込み先） | `harness-seed` |
| `wing_from_cwd` | 起動ディレクトリ名 → `wing_{project}` | `true` |
| `init_wing_if_missing` | 初回に wing が無ければシード drawer で作成 | `true` |
| `wing` | 明示プロジェクトキー（最優先） | `null` |
| `room` | エージェント room の明示 override | `null` |

レイアウト: **`wing_{project}` はプロジェクト共有**、その中の **`room={agent_name}` がエージェント固有**。検索は wing 全体（他エージェントの room も見える）。diary は自 room のみ。

| キー | 意味 | 既定 |
|------|------|------|
| `timeout_secs` | 呼び出しタイムアウト | `30` |
| `base_url` | HTTP モード用 | `http://127.0.0.1:8765` |

別ディレクトリで起動すると wing が変わり、検索・diary が混ざらない。`HARNESS_WORKSPACE` があればそのディレクトリ名を使う。固定したいときは `"wing": "OpenHarness"` を明示。

環境変数: `HARNESS_SEED_MEMPALACE_COMMAND` / `MEMPALACE_COMMAND`、`HARNESS_SEED_MEMPALACE_AGENT`。

実装の正本: [doc/ja/architecture/03_記憶層.md](../doc/ja/architecture/03_記憶層.md)。

例 — local のみ:

```json
"memory": {
  "local": true,
  "backends": [],
  "recent_work": { "enabled": true, "max_entries": 3, "max_chars": 800 },
  "search": { "enabled": true, "top_k": 5, "max_chars": 3200 },
  "rag": { "router": "llm", "max_queries": 3 }
}
```

例 — local + mempalace（Cursor と同じ MCP stdio）:

```json
"memory": {
  "local": true,
  "backends": ["mempalace"],
  "providers": {
    "mempalace": {
      "protocol": "mcp_stdio",
      "command": "C:\\Python312\\python.exe",
      "args": ["-m", "mempalace.mcp_server"],
      "agent_name": "harness-seed"
    }
  }
}
```

旧 `provider: "mempalace"` も **local+mempalace** として解釈します（local を消さない）。`provider: "noop"` はレイヤなし。

新しいバックエンド: `MemoryBridge` 実装 → `factory` の `build_backend` に登録 → `backends` と `providers.<名前>` を書く。


## `tools` セクション（組み込みツール）

| キー | 意味 |
|------|------|
| `tools.packs` | パックの ON/OFF オブジェクト（例: `{ "basic": true, "coding": true, "web_search": false }`）。`"true"` / `"false"` 文字列も可。未設定・`{}` 時は `basic`+`coding`、Brave キーがあり `web_search` が明示 `false` でなければ `web_search` を自動追加。旧形式 `["basic","coding"]` も可 |
| `tools.brave_search.api_key` | Brave Search API キー（`null` なら `BRAVE_SEARCH_API_KEY`） |
| `tools.brave_search.max_results` | `web_search` の既定件数（1–20、既定 `5`） |
| `tools.brave_search.fetch_content` | スニペットが空のとき結果 URL の本文を取得（既定 `false`） |
| `tools.brave_search.max_content_chars` | 本文取得の上限（既定 `2048`） |

ReAct の `web_search` ツールが有効になるのは API キーが解決できたときのみ。起動ログに `tools: web_search (Brave Search API)` が出る。

## `react` セクション（ループ・短期記憶）

`config/config.json` の `react` で ReAct の上限を変更します（`main` / ライブラリの `AppConfig::react_config` 経由）。

| キー | 意味 | 既定 |
|------|------|------|
| `max_steps` | **1 回の REPL 入力**あたりの最大 `decide` 回数（Thought/Action のループ。溢れたら一度 answer を強制し、だめなら trace 根拠でフォールバック） | `16` |
| `session_max_turns` | **完了ターン**を `Previous turns` に残す件数（超過分は古い順に破棄） | `8` |
| `verbose` | Thought/Action/Observation を stderr に出す（CLI の `-v` でも ON） | `false` |
| `show_prompt` | 各 ReAct ステップのプロンプト全文を stderr に出す（CLI の `--show-prompt` でも ON） | `false` |

起動時に OS / シェルは自動検出され、stderr に `runtime: ...` と LLM プロンプトの `Execution environment` に反映されます（`src/runtime.rs`）。

| `show_plan` | `two_phase` 時に計画を stdout に表示（既定 `true`） |
| `show_task_execution` | サブタスクごとの契約ツール列・実行後の実ツール列（既定 `true`） |
| `show_tool_output` | 各ツールのコマンド・結果を stderr に表示（`run_cmd` は `$ command` 形式、既定 `true`） |
| `advance.enabled` | 外側推進ループ（計画→フェーズ逐次、`recalled` 引き継ぎ）。`true` 時は `two_phase` より優先 | `false` |
| `advance.max_phases` | 1 リクエストの最大フェーズ数 | `8` |
| `advance.clear_session_each_phase` | 各フェーズ前に REPL 短期記憶をクリア | `true` |
| `advance.max_note_chars` | 完了フェーズ要約の `recalled` 上限文字数 | `1500` |
| `advance.show_phases` | 各フェーズ開始を stdout に表示 | `true` |
| `advance.min_substantive_obs` | 判定前に必要な実質証拠（read/grep 等）成功 observation 数。浅い `list_dir` は数えない | `3` |
| `advance.citation_check` | 最終合成後にパス引用を先行 Paths と照合し、無いものを未検証注記 | `true` |
| `advance.claim_check` | 結論・合成前に先行 Claims の否定証拠を一度探す | `true` |
| `advance.absence_check` | 最終回答の不在主張を trace と照合し、未検証・矛盾を注記 | `true` |
| `show_context_metrics` | ターン終了時に `[context turn]` を stderr に出す | `true` |
| `arg_audit_mode` | タスク契約の引数監査: `off` / `soft`（既定・警告のみ）/ `hard`（不一致で失敗） | `soft` |

推進ループ: [doc/ja/architecture/07_推進ループ.md](../doc/ja/architecture/07_推進ループ.md)

例:

```json
"react": {
  "max_steps": 24,
  "session_max_turns": 12,
  "verbose": false,
  "show_context_metrics": true
}
```

REPL の往復回数自体に上限はありません。短期記憶だけリセットする場合は REPL で `clear`。

## `prompt` セクション（コンテキストブロック）

`prompt.rules_paths` で **追加ルール**（Markdown）を system プロンプトへ注入します。組み込み時は `PromptBlocks` を直接編集しても同じです。

| キー | 意味 |
|------|------|
| `rules_paths` | ファイルまたはディレクトリのパス配列。ディレクトリの場合は直下の `*.md` のみ読み込み |

例:

```json
"prompt": {
  "rules_paths": [".agent/rules"]
}
```

ライブラリ API:

- `PromptBlocks::push_rule` / `push_recalled` — 実行中に追記
- `TurnPromptContext::render()` — `system` + `user` メッセージ列
- `ReActLoop::blocks` — REPL / 組み込み側から参照・変更
