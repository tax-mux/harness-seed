# ideas（未実装・検討中の設計メモ）

実装が入ったら [`architecture/`](../architecture/README.md) の番号付き章へ統合し、ここは削除するかリダイレクトスタブにする。

| 区分 | 置き場 |
|------|--------|
| 索引 | [`doc/README.md`](../../README.md) |
| 言語ホーム | [`ja/README.md`](../README.md) |
| 実装済み・現行仕様 | [`architecture/`](../architecture/README.md) |
| アイディア・検討中 | **`doc/ja/ideas/`**（このフォルダ） |

## 検討中

| ドキュメント | 概要 |
|--------------|------|
| [tool-attention-reuse-ideas.md](tool-attention-reuse-ideas.md) | Tool Attention（要約プール → promote → 幻覚ゲート） |
| [shell-hook-rtk.md](shell-hook-rtk.md) | `run_cmd` の ShellHook チェーン、RTK を PreCommand で載せる案 |
| [corpus2skill-integration.md](corpus2skill-integration.md) | Corpus2Skill（ナビ型 Skill ツリー）。知識想起の代替候補 |
| [context-colormap.md](context-colormap.md) | カラーマップ HTML / ヒートマップ等（stderr v0 は実装済み） |

## 正本へ移動済み（スタブ）

| スタブ | 正本 |
|--------|------|
| [task-registry.md](task-registry.md) | [../architecture/05_タスクレジストリ.md](../architecture/05_タスクレジストリ.md) |
| [tool-plugins.md](tool-plugins.md) | [../architecture/06_ツールプラグイン.md](../architecture/06_ツールプラグイン.md) |
| [mempalace-integration.md](mempalace-integration.md) | [../architecture/03_記憶層.md](../architecture/03_記憶層.md) |
| [memory-and-replan-architecture.md](memory-and-replan-architecture.md) | [../architecture/03_記憶層.md](../architecture/03_記憶層.md) |
