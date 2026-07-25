# Architecture (English)

HarnessSeed is an **embeddable agent execution engine** for existing apps (not a chat UI). One user request (a turn) roughly goes:

1. Recall past work or knowledge if needed (memory)
2. Decide what to do in what order (planning)
3. Use tools and answer (execution)
4. Persist a record and optionally notify the host (diary / hooks)

Principles: [development-principles.md](../development-principles.md) (openings state what and why in prose) · Ideas: [ideas/](../ideas/README.md) · Japanese: [ja/architecture/README.md](../../ja/architecture/README.md)

## Start here

1. [Glossary](glossary.md) — plain definitions of insider words
2. [00 harness-seed structure](00_harness-seed-structure.md) — the whole story
3. [01 Planning](01_planning-layer.md) and [02 Execution](02_execution-layer.md) — the core two stages
4. Then [03 Memory](03_memory-layer.md) and [04 Host extensions](04_host-extensions.md) as needed
5. Contracts, long jobs, and wire protocol: 05+ in the table below

## Chapters

| # | Document | Content (plain) |
|---|----------|-----------------|
| — | [Glossary](glossary.md) | Definitions of common terms |
| 00 | [harness-seed structure](00_harness-seed-structure.md) | How a request becomes a final answer; execution modes |
| 01 | [Planning layer](01_planning-layer.md) | How the subtask list (work orders) is designed |
| 02 | [Execution layer](02_execution-layer.md) | How tools run against the plan and produce an answer |
| 02-01 | [Tool selection](02-01_tool-selection.md) | Which tools are available |
| 03 | [Memory layer](03_memory-layer.md) | How past work / knowledge is loaded and saved |
| 04 | [Host extensions](04_host-extensions.md) | Side effects (e.g. tickets) without changing the core path |
| 05 | [Task registry](05_task-registry.md) | Predefined work as JSON contracts |
| 06 | [Tool plugins](06_tool-plugins.md) | How tool packs are plugged in |
| 07 | [Advance loop](07_advance-loop.md) | Splitting long requests into phases |
| 08 | [ReAct implementation](08_react-implementation.md) | Loop mechanics and limits |
| 09 | [Context mapping](09_context-memory-mapping.md) | What goes into the prompt, and at what length |
| 10 | [Minimum action unit](10_agent-minimum-action-unit.md) | One “action” = one tool call |
| 11 | [Wire protocol](11_wire-protocol.md) | JSON Lines contract with the host |

- Overview (SVG): [full_agent_architecture_v2.svg](../../ja/architecture/full_agent_architecture_v2.svg)
- Language home: [../README.md](../README.md)
- Doc index: [../../README.md](../../README.md)
- Built-in tools: [../builtin_tools/README.md](../builtin_tools/README.md)
