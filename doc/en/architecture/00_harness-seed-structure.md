# harness-seed Structure

## What this is

HarnessSeed is an **embeddable agent execution engine** for host apps. One user request roughly goes:

1. (Optional) recall past work or knowledge  
2. **Plan**: decide what to do in what order (no tools here)  
3. **Execute**: work through the plan, use tools if needed, return a final answer  
4. Persist a record; the host may attach side effects such as ticket updates

Planning and execution share the same “think → (optionally) act” loop primitive, but differ in **whether tools are enabled** and **output shape** (a plan vs a user-facing answer).

Glossary: [glossary.md](glossary.md) · Principles: [development-principles.md](../development-principles.md)

## When to read this

- You want a map of the whole repository (start here)
- You need to explain why planning and execution are separate

Contracts and settings live in the linked chapters below.

## Plain flow

```mermaid
flowchart TB
    A["User request"] --> M["Load memory"]
    M --> B["Make a plan"]
    B --> C{"Answer without tools?"}
    C -->|yes| D["Final answer"]
    C -->|no| E["Run work items"]
    D --> F["Record and host notify"]
    E --> F
```

The same flow with implementation symbols is in the next section.

Related:

- Memory: [03_memory-layer.md](03_memory-layer.md)
- Host side effects: [04_host-extensions.md](04_host-extensions.md)
- Overview (SVG): [full_agent_architecture_v2.svg](../../ja/architecture/full_agent_architecture_v2.svg)
- Index: [README.md](README.md)
- Minimum action unit: [10_agent-minimum-action-unit.md](10_agent-minimum-action-unit.md)
- ReAct implementation: [08_react-implementation.md](08_react-implementation.md)
- Advance loop: [07_advance-loop.md](07_advance-loop.md)
- Task registry: [05_task-registry.md](05_task-registry.md)
- Planning: [01_planning-layer.md](01_planning-layer.md) ([JP](../../ja/architecture/01_計画層.md))
- Execution: [02_execution-layer.md](02_execution-layer.md) ([JP](../../ja/architecture/02_実行層.md))
- Japanese: [00_harness-seedの構造.md](../../ja/architecture/00_harness-seedの構造.md)

## 1. Overall flow (implementation)

```mermaid
flowchart TB
    A["Prompt intake<br/>run_turn(user_input)"] --> H0["on_turn_started<br/>HostScratch"]
    H0 --> M["Memory RAG<br/>work_log / knowledge"]
    M --> B["Planning layer<br/>run_plan_layer"]
    B --> H1["on_plan_finished"]
    H1 --> C{"skip_execution?"}
    C -->|yes| D["Final answer<br/>(skip execution)"]
    C -->|no| E["Execution layer<br/>per subtask:<br/>on_subtask_* + run"]
    D --> F["End<br/>TurnResult + diary<br/>on_turn_finished"]
    E --> F
```

Memory details: [03_memory-layer.md](03_memory-layer.md). Host side effects: [04_host-extensions.md](04_host-extensions.md).

Opening comment in `src/plan.rs`:

> Serial orchestration: planning layer (ReAct-derived loop, no tools) → execution layer (ReAct + tools).

## 2. Role of Each Layer

| Layer | Entry | Brain | Loop | Tools | Termination |
|-------|-------|-------|------|-------|-------------|
| **Planning** | `run_plan_layer` | `PlanBrainMode` | `run_layer_loop` (`LayerLoopOptions::plan`) | **none** | `Answer` → `PlanArtifact` |
| **Execution** | `run_turn_two_phase` / `run_subtask_exec_audited` | exec `BrainMode` | `run_layer_loop` (`LayerLoopOptions::exec`) or **step driver** | **yes** | `Answer` → user-facing response |

### Planning Layer Output (PlanArtifact)

The planning layer parses JSON returned by the LLM and builds an ordered list of subtasks.

```json
{
  "summary": "…",
  "skip_execution": false,
  "subtasks": [
    { "id": 1, "goal": "…", "done_when": "…" }
  ]
}
```

- `skip_execution: true` — trivial Q&A (greetings, help) that needs no tools
- Subtasks may reference registered task ids from `tasks/*.json`

### Execution Layer Behavior

Each subtask runs via one of:

1. **ReAct loop** — receives a mission built by `format_mission`; repeats `Thought → Action → Observation`
2. **Step driver** — when a registered task has a `steps[]` contract, runs `execute_action` in contract order without an LLM (`react.use_step_driver` defaults to `true`)

## 3. Shared ReAct Loop (layer.rs)

Both layers share **`run_layer_loop` in `src/layer.rs`**.

```mermaid
flowchart TB
    subgraph shared["ReAct-derived primitives (layer.rs)"]
        LOOP["run_layer_loop"]
        TRACE["TurnTrace"]
        BRAIN["AgentBrain::decide"]
    end
    subgraph plan["Planning layer"]
        PB["PlanBrainMode"]
        OUT["PlanArtifact"]
    end
    subgraph exec["Execution layer"]
        EB["exec BrainMode"]
        TR["ToolRuntime"]
    end
    LOOP --> PB --> OUT
    LOOP --> EB --> TR
```

| Option | Planning (`plan`) | Execution (`exec`) |
|--------|-------------------|---------------------|
| `tools_enabled` | `false` | `true` |
| `context_label` | `"plan"` | `"step"` |
| `max_thoughts` | 1 (default) | 1 (default) |

**Principle**: the planning phase never touches the environment. Side effects occur only in **execution-phase `Action`s**.

## 4. Sequence Within One Turn (two_phase)

Flow when `react.two_phase: true` (default in `config/config.json`).

```mermaid
sequenceDiagram
    participant U as User input
    participant R as ReActLoop
    participant PL as PlanBrainMode
    participant E as ExecBrain (×N subtasks)

    U->>R: run_turn
    loop planning layer max_steps_plan
        R->>PL: decide (thought / answer only)
        Note over PL: Action rejected via observation
    end
    PL-->>R: PlanArtifact (parse answer)
    alt skip_execution
        R->>E: run_layer_loop (original input)
    else subtask list
        loop each subtask
            alt task has steps[] contract
                R->>R: run_subtask_driver (sequential execute_action)
            else free-form execution
                R->>E: run_layer_loop (mission)
            end
        end
    end
    R-->>U: TurnResult
```

## 5. Execution Mode Switching

`ReActLoop::run_turn` (`src/react.rs`) branches on configuration.

```mermaid
flowchart TD
    RT["run_turn(user_input)"] --> AD{"advance.enabled?"}
    AD -->|yes| ADV["run_turn_advance<br/>plan → phased execution"]
    AD -->|no| TP{"two_phase?"}
    TP -->|yes| TWO["run_turn_two_phase<br/>plan → execute"]
    TP -->|no| ONE["run_turn_single<br/>single ReAct only"]
    ADV --> END["TurnResult"]
    TWO --> END
    ONE --> END
```

| Setting | Code default (key omitted) | Behavior |
|---------|----------------------------|----------|
| `react.two_phase` | `false` | Serial plan → execution (sample config sets `true`) |
| `react.advance.enabled` | `false` | Outer advance loop (priority over `two_phase`; sample sets `true`) |
| `react.use_step_driver` | `true` | Run contract / non-`react_only` tasks without LLM |
| `react.arg_audit_mode` | `soft` | Arg audit ([task-registry.md](05_task-registry.md)) |

When `advance.enabled: true`, **advance takes priority over `two_phase`**, but both still **pass through the planning layer (`run_plan_layer`) first**.

## 6. Source Code Map

| Concept | File |
|---------|------|
| Turn entry | `src/react.rs` — `run_turn`, `run_turn_two_phase`, `run_turn_advance` |
| Planning loop | `src/layer.rs` — `run_plan_layer`, `run_layer_loop` |
| Plan JSON & contracts | `src/plan.rs`, `src/plan/parse.rs`, `src/plan/contract.rs` |
| Harness state | `src/harness/state.rs` — `HarnessState`, `PlanArtifact` |
| Execution tools | `src/tool/` — `ToolRuntime`, `execute_action` |
| Step driver | `src/tasks/driver.rs` |
| Task definitions | `tasks/*.json`, `src/tasks/registry.rs` |
| Minimum action unit | `src/action.rs` — `Action`, `Observation`, `TurnTrace` |

## 7. Hierarchy Overview

```mermaid
flowchart TB
    subgraph session["Session (REPL)"]
        T1["Turn 1"]
        T2["Turn 2"]
    end

    subgraph turn["One turn"]
        PL["Planning layer → PlanArtifact"]
        subgraph exec_turns["Execution layer (serial subtasks)"]
            E1["Execution loop ①"]
            E2["Execution loop ②"]
        end
        ANS["TurnResult.answer"]
    end

    subgraph react_loop["Inside one execution loop"]
        TH["Thought"]
        A1["Action ①"]
        OBS["Observation"]
        TXT["Answer"]
    end

    T1 --> PL
    PL --> E1
    E1 --> E2
    E2 --> ANS
    E1 --> react_loop
    TH --> A1 --> OBS
    OBS --> TH
```

| Level | HarnessSeed type | Minimum action unit? |
|-------|------------------|----------------------|
| Session | `SessionMemory` | no |
| Turn | `TurnResult` | no |
| Plan | `PlanArtifact` | no (no tools) |
| Execution loop | ReAct for one subtask | no |
| Action | `Action` + `invoke_id` | **yes** |
| Observation | `Observation` | result of an action |

## 8. Summary

- harness-seed is centered on a **two-layer model: planning + execution**.
- Both layers are ReAct-derived; the planning layer **designs subtasks without tools**, and only the execution layer touches the environment via **ToolRuntime**.
- Simple conversation can skip the execution layer via `skip_execution`.
- Registered tasks can fall through to the **step driver** (no LLM) in the execution layer.
- When `advance` is enabled, long work is split into phases while the same two-layer structure repeats, carrying `recalled` context forward.
