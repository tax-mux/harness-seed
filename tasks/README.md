# タスク定義（機能塊）

各タスクは **必須実行メソッド（組み込みツール名）** と **`order` による実行順序** を宣言する。プロンプト用の自由文テンプレートではない。

## スキーマ

```json
{
  "id": "write_file_verify",
  "summary": "一行説明（プランナー向け）",
  "default_params": { "path": "tmp/out.txt", "content": "" },
  "done_when": "完了の言語条件",
  "steps": [
    { "order": 1, "method": "write_file", "args": { "path": "{path}", "content": "{content}" }, "required": true },
    { "order": 2, "method": "read_file", "args": { "path": "{path}" }, "required": true }
  ]
}
```

| フィールド | 意味 |
|------------|------|
| `order` | 実行順（1 始まり。小さいほど先） |
| `method` | ツール名（`list_dir`, `write_file`, `web_search`, …） |

## 組み込みタスク

| id | 必須ツール順 | 用途 |
|----|-------------|------|
| `list_dir` | `list_dir` | ディレクトリ一覧 |
| `write_file_verify` | `write_file` → `read_file` | 書き込み検証 |
| `web_research` | `web_search` | Web 検索（`tools.brave_search.api_key` 必須） |
| `generic` | （なし） | 実行層 ReAct がツールを自由選択 |
| `args` | 引数テンプレート（`{param}` は `params` で展開） |
| `required` | 監査対象（既定 `true`） |
| `steps: []` | 固定順なし（`generic`） |

## 計画 JSON との接続

```json
{ "id": 1, "task": "list_dir", "params": { "path": "src" } }
```

実行後、スケルトンでは `TaskRegistry::audit_subtask` が trace 上の **ツール名の順序** を照合する（引数の厳密一致は未実装）。

実装: `src/tasks/spec.rs`（定義）, `src/tasks/audit.rs`（照合）, `src/tasks/registry.rs`（読み込み・mission 生成）。
