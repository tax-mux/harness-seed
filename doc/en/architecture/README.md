# Architecture (English)

Layer contracts and execution modes for HarnessSeed. Principles: [development-principles.md](../development-principles.md). Ideas: [ideas/](../ideas/README.md).

| # | Document | Content |
|---|----------|---------|
| 00 | [harness-seed structure](00_harness-seed-structure.md) | Overall flow, layer roles, execution modes |
| 01 | [Planning layer](01_planning-layer.md) | `run_plan_layer`, Harness parse, data contract |
| 02 | [Execution layer](02_execution-layer.md) | ReAct / step driver, audit, trace merge |
| 02-01 | [Tool selection](02-01_tool-selection.md) | Catalog, tool_policy, runtime checks |
| 03 | [Memory layer](03_memory-layer.md) | Memory RAG, Bridge, diary |
| 04 | [Host extensions](04_host-extensions.md) | Lifecycle / HostScratch |
| 05 | [Task registry](05_task-registry.md) | Task contracts and audit |
| 06 | [Tool plugins](06_tool-plugins.md) | Packs / plugins |
| 07 | [Advance loop](07_advance-loop.md) | Phased execution, `recalled` carry-forward |
| 08 | [ReAct implementation](08_react-implementation.md) | Loop details and limits |
| 09 | [Context mapping](09_context-memory-mapping.md) | Short / mid / long context layout |
| 10 | [Minimum action unit](10_agent-minimum-action-unit.md) | Action = one tool call |
| 11 | [Wire protocol](11_wire-protocol.md) | JSON Lines (`--json`) |

- Overview (SVG): [full_agent_architecture_v2.svg](../../ja/architecture/full_agent_architecture_v2.svg)
- Japanese: [Japanese architecture](../../ja/architecture/README.md)
- Doc index: [../../README.md](../../README.md)
- Language home: [../README.md](../README.md)
- Built-in tools: [../builtin_tools/README.md](../builtin_tools/README.md)
