# アーキテクチャ（日本語）

HarnessSeed の層・契約・実行モードの正本。開発方針: [development-principles.md](../development-principles.md)。未実装案: [ideas/](../ideas/README.md)。

| # | ドキュメント | 内容 |
|---|--------------|------|
| 00 | [harness-seed の構造](00_harness-seedの構造.md) | 全体フロー、層の役割、実行モード |
| 01 | [計画層](01_計画層.md) | `run_plan_layer`、Harness パース、データ契約 |
| 02 | [実行層](02_実行層.md) | ReAct / ステップドライバ、監査、trace マージ |
| 02-01 | [ツールの選択](02-01_ツールの選択.md) | catalog、tool_policy、実行時検証 |
| 03 | [記憶層](03_記憶層.md) | Memory RAG、Bridge、diary |
| 04 | [ホスト拡張](04_ホスト拡張.md) | `TurnLifecycle`、`HostScratch` |
| 05 | [タスクレジストリ](05_タスクレジストリ.md) | `tasks/*.json`、監査契約 |
| 06 | [ツールプラグイン](06_ツールプラグイン.md) | `tools.packs`、`register_plugin` |
| 07 | [推進ループ](07_推進ループ.md) | フェーズ分割、`recalled` 引き継ぎ |
| 08 | [ReAct 実装](08_ReAct実装.md) | ループ構成・制限 |
| 09 | [コンテキストマッピング](09_コンテキストマッピング.md) | 短期／中期／長期の載せ方 |
| 10 | [最少行動単位](10_最少行動単位.md) | Action = 1 ツール呼び出し |
| 11 | [ワイヤプロトコル](11_ワイヤプロトコル.md) | JSON Lines（`--json`） |

- 全体図（SVG）: [full_agent_architecture_v2.svg](full_agent_architecture_v2.svg)
- English: [en/architecture/README.md](../../en/architecture/README.md)（00–11 全文）
- 言語ホーム: [../README.md](../README.md)
- ドキュメント索引: [../../README.md](../../README.md)
- 組み込みツール仕様: [../builtin_tools/README.md](../builtin_tools/README.md)
