# Memory Layer (Memory RAG + MemoryBridge)

Implementation source of truth for how the embedded agent joins host / external memory.

- Development principles: [development-principles.md](../development-principles.md) (prefer general solutions)
- Design history (stub): [ideas/memory-and-replan-architecture.md](../../ja/ideas/memory-and-replan-architecture.md)
- mempalace adapter: [adapters/mempalace-adapter/README.md](../../../adapters/mempalace-adapter/README.md)
- Config: `memory` section in [config/README.md](../../../config/README.md)
- Japanese version: [03_記憶層.md](../../ja/architecture/03_記憶層.md)

## 1. Roles

| Layer | Responsibility |
|-------|----------------|
| **Memory RAG** (`src/memory/rag.rs`) | At turn start, branch **work log vs knowledge**, call the Bridge, build `recalled` |
| **MemoryBridge** | I/O only: `recent_work` / `search` / `diary`. Backend-specific logic stays in adapters |
| **Plan / exec** | Do not know the Bridge. Consume `recalled` and `Previous turns` only |

Never call mempalace (etc.) directly from the core. Always go through `MemoryBridge` (e.g. factory-built `LayeredMemoryBridge`).

## 2. In-turn flow

```mermaid
flowchart TB
    START["run_turn start"] --> RAG["MemoryRag::run\nroute → retrieve → pack"]
    RAG -->|work_log| RW["MemoryBridge::recent_work"]
    RAG -->|knowledge| SE["MemoryBridge::search(queries)"]
    RW --> REC["PromptBlocks.recalled"]
    SE --> REC
    RAG --> POL["SessionPromptPolicy\nPrevious turns only on work_log"]
    REC --> PLAN["Planning layer"]
    POL --> PLAN
    PLAN -->|recall step| SE2["recall_knowledge\nknowledge channel only"]
    PLAN --> EXEC["Execution layer"]
    EXEC --> ANS["Final Answer\nTurnResult.answer"]
    ANS --> DIARY["finish_turn\nMemoryBridge::diary"]
```

### 2.1 Routing

The router (`memory.rag.router`: `llm` default / `rule`) returns a JSON-like branch.

| Field | Meaning |
|-------|---------|
| `work_log` | Load recent diary (continued work) |
| `knowledge` | Run knowledge search |
| `queries` | Search terms for knowledge (1…`max_queries`) |

**If both are true, prefer knowledge and drop `work_log`** (mechanical guard against topic mixing).

- Work log: continuation cues (“continue”, “earlier”, …)
- Knowledge: facts, explanations, unrelated questions
- Neither: greetings, etc.

`Previous turns` (`SessionMemory`) is injected **only on `work_log`** (`SessionPromptPolicy`).

### 2.2 Layer stack

| Layer | Implementation | Role |
|-------|----------------|------|
| local | `LocalDiaryBridge` | In-process diary (recommended always-on) |
| mempalace etc. | `MempalaceBridge` + adapter | Persistence / cross search (add via `backends`) |

Do not replace `local` with an external backend. If externals fail, local still works.

### 2.3 Diary write

When the turn reaches a **final answer** (`TurnResult.answer` after execution — not the planning layer’s `step:answer`), `finish_turn` → `record_diary` → `MemoryBridge::diary`.

- local: full-text entry
- mempalace: compressed text (`user` / `summary` / `phases` / **always `answer`**) written to its own room with `source_file=*:diary`

Success log: `[memory] diary written: …`

## 3. Contract with the planning layer

| Item | Behavior |
|------|----------|
| `knowledge_sufficient` | Whether Recalled and/or general knowledge can fully answer. Included in plan JSON |
| `skip_execution: true` | **Allowed only when `knowledge_sufficient == true`**. Otherwise the harness forces execution |
| Empty steps but execution needed | One freeform subtask (no hardcoded tool recipes) |
| `recall` step | Planning layer only. Extra knowledge search (`recall_max_rounds`, default 2) |
| Plan loop cap | `react.max_steps_plan` (default **4**). Do not explore for long |
| Missing answer | Once force “emit a problem-solving answer”; if still missing → `single_subtask(user request)` |

The planning layer has no tools (`recall` is the only exception). Gather missing evidence in the **execution layer**.

## 4. Code layout

| Path | Content |
|------|---------|
| `src/memory/mod.rs` | `MemoryBridge`, local, config, factory entry |
| `src/memory/rag.rs` | `MemoryRag`, `RuleRouter` / `LlmRouter`, injection |
| `src/memory/layered.rs` | Stack of Bridges |
| `src/memory/mempalace.rs` | Thin adapter wrapper (feature `mempalace`) |
| `src/memory/factory.rs` | `config.memory` → Bridge |
| `src/action.rs` | Observation char cap (avoid blowing context on huge files) |
| `src/tool/builtin.rs` | If `read_file` gets a directory, suggest `list_dir` |

## 5. Config example

```json
"memory": {
  "local": true,
  "backends": ["mempalace"],
  "providers": {
    "mempalace": {
      "protocol": "mcp_stdio",
      "command": "python",
      "args": ["-m", "mempalace.mcp_server"],
      "agent_name": "harness-seed",
      "wing_from_cwd": true
    }
  },
  "recent_work": { "enabled": true, "max_entries": 3, "max_chars": 800 },
  "search": { "enabled": true, "top_k": 5, "max_chars": 3200 },
  "rag": { "router": "llm", "max_queries": 3 },
  "recall_max_rounds": 2
}
```

Key details: [config/README.md](../../../config/README.md).
