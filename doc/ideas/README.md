# ideas（未実装の設計メモ）

HarnessSeed に**まだ入っていない**機能・流用案を置く。実装が入ったら該当メモを `doc/` 直下の正本に統合し、ここからは削除または「実装済み」へ移す。

| 区分 | 置き場 |
|------|--------|
| 実装済み・現行仕様 | [`doc/`](../)（このフォルダの親） |
| アイディア・検討中 | **`doc/ideas/`**（このフォルダ） |
| 論文 PDF などローカル資料 | [`doc/knowledge/`](../knowledge/)（gitignore、README のみ追跡） |

## 一覧

| ドキュメント | 概要 |
|--------------|------|
| [tool-attention-reuse-ideas.md](tool-attention-reuse-ideas.md) | Tool Attention 論文の流用（`tool_attention` モジュール案） |
| [shell-hook-rtk.md](shell-hook-rtk.md) | 汎用 ReAct の `run_cmd` に ShellHook チェーン、RTK を PreCommand で載せる案 |
| [mempalace-integration.md](mempalace-integration.md) | mempalace（中期・長期記憶）を `recalled` / `memory_bridge` 経由で接続する案 |
| [corpus2skill-integration.md](corpus2skill-integration.md) | Corpus2Skill（ナビ型 Skill ツリー）。telospvl/mempalace 代替候補 |
| [context-colormap.md](context-colormap.md) | プロンプト・ブロック別カラーマップ（v0 実装済み + HTML 等は将来） |
| [task-registry.md](task-registry.md) | 機能塊タスク `tasks/*.json` + `TaskRegistry`（スケルトン実装済み） |
