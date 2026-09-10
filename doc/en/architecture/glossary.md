# Glossary

Plain-language definitions for terms used in HarnessSeed architecture docs. Type and function names are listed under “Typical symbols.” Details live in each chapter.

Audience: people embedding HarnessSeed in a host app, or reading this repository for the first time.

Index: [README.md](README.md) · Japanese: [用語集.md](../../ja/architecture/用語集.md)

---

## Product and actors

### HarnessSeed

An **embeddable agent execution engine** (a “seed”) for existing applications. It is **not** a chat UI. The same crate provides a library and a CLI (`harness-seed`).

- Details: [00_harness-seed-structure.md](00_harness-seed-structure.md)
- Typical symbols: crate `harness-seed`, `ReActLoop`

### Host

The application that calls HarnessSeed (mail triage, coding agents, and so on). Domain rules, external APIs, and ticket systems live on the host. The engine does not know product names.

- Details: [04_host-extensions.md](04_host-extensions.md)

### Turn

Everything that happens for **one user request** (memory inject → plan → execute → final answer → diary, and so on). The next request is a new turn.

- Typical symbols: `ReActLoop::run_turn`, `TurnResult`

---

## Layers (roles)

### Planning layer

Reads the request and decides **what to do in what order** (a list of subtasks). No tools; no environment side effects.

- Details: [01_planning-layer.md](01_planning-layer.md)
- Typical symbols: `run_plan_layer`, `PlanArtifact`

### Execution layer

Follows the plan, **uses tools**, and produces the user-facing answer. Advances one subtask at a time (or by dependency waves).

- Details: [02_execution-layer.md](02_execution-layer.md)
- Typical symbols: `run_layer_loop` (exec), `run_subtask_exec_audited`

### Memory layer

Before and after a turn, **loads past work or knowledge into the prompt** and **persists a record** when done. The core never talks to an external DB directly; it goes through a bridge.

- Details: [03_memory-layer.md](03_memory-layer.md)
- Typical symbols: Memory RAG, `MemoryBridge`, `recalled`, `diary`

---

## How execution works

### ReAct (in this repository)

A loop of “think → (optionally) call a tool → observe → think again.” Planning and execution share the same loop primitive, but planning has no tools and outputs a plan; execution has tools and outputs a user answer.

- Details: [08_react-implementation.md](08_react-implementation.md), [10_agent-minimum-action-unit.md](10_agent-minimum-action-unit.md)
- Typical symbols: `run_layer_loop`, `AgentStep`

### Subtask

A **small unit of work** cut by the plan (goal + done-when). The execution layer runs these in order (or by dependencies).

- Typical symbols: `Subtask`, `PlanArtifact.subtasks`

### Task registry

A store of **predefined work contracts** (JSON): tool order, argument shapes, and so on. When a plan points at a contract task, a mechanical **step driver** can run it instead of freeform LLM tool choice.

- Details: [05_task-registry.md](05_task-registry.md)
- Typical symbols: `tasks/*.json`, `TaskRegistry`

### Step driver

Runs a contract task **by fixed steps** without asking the LLM to pick tools each time. Failures and argument drift are caught by audit.

- Details: [02_execution-layer.md](02_execution-layer.md), [05_task-registry.md](05_task-registry.md)
- Typical symbols: `run_subtask_driver`, `ArgAuditMode`

### Audit

Mechanical checks after execution: “were the contracted tools called?” “were arguments sufficient?” On failure the same subtask may be retried.

- Typical symbols: `audit_trace`, `TaskExecutionAudit`

### two_phase

The standard turn shape: **plan first, then execute** (two stages).

- Details: [00_harness-seed-structure.md](00_harness-seed-structure.md)
- Typical symbols: config `react.two_phase`, `run_turn_two_phase`

### Advance loop

Splits a long request into **multiple phases**, carries summaries of finished phases into the next prompt. Similar to `two_phase`, with thicker hand-off between phases.

- Details: [07_advance-loop.md](07_advance-loop.md)
- Typical symbols: config `react.advance`, `run_turn_advance`

---

## Memory vocabulary

### recalled

The place that holds **snippets shown to the LLM this turn** (recent work, search hits, and so on). Part of the prompt.

- Details: [03_memory-layer.md](03_memory-layer.md), [09_context-memory-mapping.md](09_context-memory-mapping.md)
- Typical symbols: `PromptBlocks.recalled`

### diary

A **work record** written at turn end (summary, answer, and so on). Becomes material for “recent work” on later turns.

- Typical symbols: `MemoryBridge::diary`, `DiaryEntry`

### MemoryBridge

The **I/O mouth** to external memory backends (local diary, mempalace, and so on). The engine does not call product-specific APIs directly.

- Typical symbols: `MemoryBridge`, `LayeredMemoryBridge`

---

## Host integration vocabulary

### Hook (lifecycle hook)

A **host callback** the engine invokes at fixed points in a turn. It does not steer planning or execution; it only carries side effects such as creating tickets.

- Details: [04_host-extensions.md](04_host-extensions.md) (“What is a hook?”)
- Typical symbols: `TurnLifecycle`

### TaskTracking

A **start / finish work-item API** aimed at PM / ticket sync. Prefer implementing this over raw hooks.

- Typical symbols: `TaskTracking`, `lifecycle_from_tracking`

### HostScratch

A per-turn host-only bag (JSON) for ticket IDs and similar. **Never included in the LLM prompt.**

- Typical symbols: `HostScratch`, `HostView`

### RunStatus / outcome

Structured result for whether a subtask or turn **completed, failed, or was cancelled**. Attached to finished hooks.

- Typical symbols: `RunStatus`, `SubtaskOutcome`, `TurnOutcome`

---

## Other

### Tools / catalog

Operations the execution layer may call (read file, run command, and so on) and their catalog text. The planning layer normally does not use them.

- Details: [02-01_tool-selection.md](02-01_tool-selection.md), [06_tool-plugins.md](06_tool-plugins.md)
- Typical symbols: `ToolRuntime`, `tools.packs`

### Wire protocol

The **one-JSON-object-per-line** contract for talking to a host via CLI `--json` and similar.

- Details: [11_wire-protocol.md](11_wire-protocol.md)
