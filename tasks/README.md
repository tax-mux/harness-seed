# タスク定義（機能塊）

実行で何度も同じ道具の順を踏む作業を、JSON の契約として置いておく。毎回 LLM に順を任せると定型でも抜けや引数ずれが起きやすいので、必須ツールと順序を機械が読める形にし、計画の候補選定・実行後の監査・（任意の）ステップドライバで共有する。ドメイン専用の手順書はエンジンにハードコードせず、`tasks/*.json` に置く。

再現したい定型や監査したいときに使う。毎回違う自由記述だけなら `generic` や freeform で足りる。用語と層の説明は [用語集](../doc/ja/architecture/用語集.md)・[タスクレジストリ](../doc/ja/architecture/05_タスクレジストリ.md)。

現行方針では **`react_only: true` が既定**で、`steps[]` は計画・監査用の契約。実行層では ReAct がツールを選ぶ。ステップドライバ（LLM なし固定順）は `react_only: false` のときだけ使う例外経路。

## スキーマ

```json
{
  "id": "write_file_verify",
  "summary": "一行ラベル",
  "planner_summary": "計画候補選定用（約200字。いつ選ぶ／選ばない）",
  "react_only": true,
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
| `summary` | 短いラベル |
| `planner_summary` | 計画候補選定用（約200字。いつ選ぶ／選ばない） |
| `react_only` | `true` なら実行は ReAct（推奨）。`false` なら step ドライバ可 |
| `steps` | 必須ツール順（監査・mission 表示）。空なら自由実行 |
| `order` / `method` / `args` / `required` | 各ステップの定義 |

## 組み込みタスク

| id | 必須ツール順 | 実行 | 用途 |
|----|-------------|------|------|
| `list_dir` | `list_dir` | ReAct | ディレクトリ一覧 |
| `write_file_verify` | `write_file` → `read_file` | ReAct | 書き込み検証 |
| `web_research` | `web_search` | ReAct | Web 検索（Brave API キー必須） |
| `process_data` | `process_data` | ReAct | 外部プラグイン（登録時のみ） |
| `generic` | （なし） | ReAct | 自由選択 |

計画フェーズは `planner_summary` を見て候補を選び、詳細カタログをコンテキストへ登録してから PROCEDURE を書く。

## 計画 JSON との接続

```json
{ "id": 1, "task": "list_dir", "params": { "path": "src" } }
```

実行後、`TaskRegistry::audit_subtask` が trace 上の **ツール名の順序**と（設定に応じて）**引数**を照合する。  
必須 method が実行ツールに無いタスクは、計画解決時に自由記述へ落とされる。

実装: `src/tasks/spec.rs`（定義）, `src/tasks/audit.rs`（照合）, `src/tasks/registry.rs`（読み込み・mission 生成）, `src/plan/candidates.rs`（候補選定）。
