# アーキテクチャ（日本語）

HarnessSeed の二層モデル（計画層・実行層）とツール選定の解説。

| # | ドキュメント | 内容 |
|---|--------------|------|
| 00 | [harness-seed の構造](00_harness-seedの構造.md) | 全体フロー、層の役割、実行モード |
| 01 | [計画層](01_計画層.md) | `run_plan_layer`、Harness パース、データ契約 |
| 02 | [実行層](02_実行層.md) | ReAct / ステップドライバ、監査、trace マージ |
| 02-01 | [ツールの選択](02-01_ツールの選択.md) | catalog、tool_policy、実行時検証 |

- 全体図（SVG）: [full_agent_architecture_v2.svg](../full_agent_architecture_v2.svg)
- English: [architecture-en/README.md](../architecture-en/README.md)
