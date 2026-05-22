# 組み込みツール一覧

HarnessSeed が `ToolRuntime`（`src/tool/`）に実装しているツール。LLM 頭脳は各ターンで `{"step":"action","tool":"<name>","args":{...}}` として 1 回ずつ呼び出す。有効なツールは `tools.packs` で切り替える（[tool-plugins.md](../ideas/tool-plugins.md)）。

## ドキュメント

| ツール | ファイル | 概要 |
|--------|----------|------|
| `echo` | [echo.md](echo.md) | 文字列をそのまま返す |
| `time` | [time.md](time.md) | Unix 時刻（秒） |
| `list_dir` | [list_dir.md](list_dir.md) | ディレクトリ一覧 |
| `grep` | [grep.md](grep.md) | ワークスペース内テキスト検索 |
| `read_file` | [read_file.md](read_file.md) | ファイル読み取り |
| `write_file` | [write_file.md](write_file.md) | ファイル書き込み |
| `run_cmd` | [run_cmd.md](run_cmd.md) | シェルコマンド実行 |
| `web_search` | [web_search.md](web_search.md) | Brave Search API による Web 検索 |

## 共通ルール

- **実装**: `src/tool/builtin.rs` + `ToolRegistry`
- **カタログ**: `ToolRegistry::format_catalog()` → `PromptBlocks.tool_catalog`
- **システムプロンプト**: `src/llm/brain.rs` の `SYSTEM_PROMPT` にも列挙（追加時は両方を更新）
- **パス制限**: `read_file` / `write_file` / `list_dir` / `grep` / `run_cmd` のパスは [ワークスペース](#ワークスペース) 内のみ

### ワークスペース

| 項目 | 値 |
|------|-----|
| ルート | クレートルート（`CARGO_MANIFEST_DIR` = 通常はリポジトリ直下） |
| 解決 | `resolve_in_workspace(path)` |
| 禁止 | 絶対パス、`..` によるルート外への脱出 |

### Observation

| フィールド | 意味 |
|------------|------|
| `ok: true` | 成功。`output` に結果テキスト |
| `ok: false` | 失敗。`output` にエラー説明 |

未知の `tool` 名は `unknown tool: <name>` で失敗する。

## 関連

- [../react-implementation.md](../react-implementation.md) — ReAct ループ全体
- [../agent-minimum-action-unit.md](../agent-minimum-action-unit.md) — 最少行動単位（1 Action）
