# 設定ファイル

## レイアウト

| パス | 役割 |
|------|------|
| `config/config.json` | **実行時に読む正本**（ここを編集・上書きする） |
| `config/samples/config.*.json` | コネクタ別のひな形（リポジトリに固定） |

## プロバイダの切り替え

使いたいサンプルを `config.json` にコピーして上書きします。

```bash
# Ollama（既定と同じ内容）
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
| `max_steps` | **1 回の REPL 入力**あたりの最大 `decide` 回数（Thought/Action のループ。溢れたら `MaxStepsExceeded`） | `16` |
| `session_max_turns` | **完了ターン**を `Previous turns` に残す件数（超過分は古い順に破棄） | `8` |
| `verbose` | Thought/Action/Observation を stderr に出す（CLI の `-v` でも ON） | `false` |
| `show_prompt` | 各 ReAct ステップのプロンプト全文を stderr に出す（CLI の `--show-prompt` でも ON） | `false` |

起動時に OS / シェルは自動検出され、stderr に `runtime: ...` と LLM プロンプトの `Execution environment` に反映されます（`src/runtime.rs`）。

| `show_plan` | `two_phase` 時に計画を stdout に表示（既定 `true`） |
| `show_task_execution` | サブタスクごとの契約ツール列・実行後の実ツール列（既定 `true`） |
| `show_tool_output` | 各ツールのコマンド・結果を stderr に表示（`run_cmd` は `$ command` 形式、既定 `true`） |
| `scout.enabled` | 計画前スカウト（情報評価・ツール収集 → `recalled`） | `false` |
| `scout.max_steps` | スカウト ReAct の最大ステップ | `6` |
| `scout.skip_trivial` | `help` / `time` / `echo` でスカウト省略 | `true` |
| `advance.enabled` | 外側推進ループ（計画→フェーズ逐次、`recalled` 引き継ぎ）。`true` 時は `two_phase` より優先 | `false` |
| `advance.max_phases` | 1 リクエストの最大フェーズ数 | `8` |
| `advance.clear_session_each_phase` | 各フェーズ前に REPL 短期記憶をクリア | `true` |
| `advance.max_note_chars` | 完了フェーズ要約の `recalled` 上限文字数 | `1500` |
| `show_context_metrics` | ターン終了時に `[context turn]` を stderr に出す | `true` |

スカウト: [doc/scout-phase.md](../doc/scout-phase.md) — 推進ループ: [doc/advance-loop.md](../doc/advance-loop.md)

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
