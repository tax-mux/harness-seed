# Minimum action unit


Treat the smallest real-world effect as one tool call. If “action” means different things per implementation, logs, audit, and UI diverge. Reasoning text and plan JSON are not actions; tool execution and Observation are the unit.

Think → call one tool → observe → think again or answer.

Glossary: [glossary.md](glossary.md) · ReAct: [08_react-implementation.md](08_react-implementation.md) · [JP](../../ja/architecture/10_最少行動単位.md)

## 1. Hierarchy overview

```mermaid
flowchart TB
    subgraph session["Session (REPL)"]
        T1["Turn 1"]
        T2["Turn 2"]
    end

    subgraph turn["One turn (one user utterance → one response)"]
        direction TB
        PL["Plan (PlanArtifact / text JSON)"]
        subgraph exec_turns["Execution phase (subtasks serial)"]
            E1["Execution loop ①"]
            E2["Execution loop ②"]
        end
        ANS["User-facing Answer"]
    end

    subgraph react_loop["Inside one execution loop"]
        TH["Thought"]
        A1["Action ①"]
        A2["Action ②"]
        TXT["Subtask Answer"]
    end

    subgraph atomic["Minimum action unit = 1 Tool Call"]
        TC["tool + args"]
        EX["ToolRuntime execute"]
        OBS["Observation"]
    end

    T1 --> PL
    PL --> E1
    E1 --> E2
    E2 --> ANS
    E1 --> react_loop
    A1 --> TC --> EX --> OBS
    OBS --> TH
    TH --> A2
```

A session is a sequence of turns. Each turn first plans the work, then runs any needed subtasks in order. Planning itself does not change the outside world.

Inside a subtask, the agent may think, call one tool, observe the result, and decide again. That single tool call—and the observation it produces—is the unit used for audit and logs.

| Level | Example | Minimum unit? | HarnessSeed (`two_phase` on) |
|-------|---------|---------------|------------------------------|
| Session | Entire chat | × | `SessionMemory` (REPL short-term) |
| Turn | One user utterance → one response | × | `run_turn` / `TurnResult` |
| Plan | Subtask list JSON | × | `PlanArtifact` (**no tools**) |
| Execution loop | ReAct for one subtask | × | `run_turn_single(mission)` |
| Action | One `read_file` | **◎** | `Action` + `invoke_id` |
| Observation | File contents · exit code | Result of action | `Observation` |

**Principle**: the planning phase does not touch the environment. Side effects come **only from `Action` in the execution phase**.

## 2. ReAct loop (inside one action)

"Think → run one tool → see result" is the smallest loop in the **execution phase**.

```mermaid
sequenceDiagram
    participant U as User / mission
    participant M as AgentBrain
    participant T as ToolRuntime

    U->>M: TurnPromptContext
    M->>M: Thought (no side effects)
    M->>T: Action: tool(args)  ← minimum action unit
    T->>M: Observation
    alt still insufficient
        M->>T: Action: next tool
        T->>M: Observation
    end
    M->>U: Answer (subtask complete)
```

The brain first considers the mission without changing anything. It then performs one tool call and receives its observation.

If more evidence or work is needed, it repeats with another action. Otherwise it returns an answer, which completes the subtask without a side effect.

| Variant | Side effects on environment | HarnessSeed type |
|---------|----------------------------|------------------|
| `Thought` | None | `AgentStep::Thought` |
| `Action` | **Yes** | `AgentStep::Action` → `execute_action` |
| `Answer` | None (text response) | `AgentStep::Answer` |

Thought and answer are control and communication steps. Only action enters the tool runtime, which is why it is the point recorded as the minimum real-world operation.

## 3. Planning layer · execution layer (`two_phase`, shared ReAct loop)

When `react.two_phase: true`, **both** planning and execution layers run via `run_layer_loop` (`src/layer.rs`).

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
            else free execution
                R->>E: run_layer_loop (mission)
            end
        end
    end
    R-->>U: TurnResult
```

Planning happens before any executable work and can only produce a structured work list. A request with sufficient knowledge takes the direct-reply branch; otherwise each planned item is handled in order.

For an item with a fixed contract, the driver runs its steps directly. For other work, the execution brain chooses actions as it gathers evidence. Both branches finish by returning one combined turn result.

| Layer | Brain | Loop | Tools | Termination |
|-------|-------|------|-------|-------------|
| Plan | `PlanBrainMode` | `run_plan_layer` | **none** | `Answer` → `PlanArtifact` |
| Exec | exec `BrainMode` | `run_layer_loop` or **step driver** | **yes** | `Answer` → user-facing |

The shared loop gives planning and execution a common control structure. The distinction is operational: planning has no tools and yields a plan, while execution has tools and yields user-facing work results.

When a subtask has a registered task id (`tasks/*.json`) with a defined `steps[]`, and `react.use_step_driver` (default `true`) is on, **`src/tasks/driver.rs`** runs `execute_action` in contract order without an LLM. On failure it falls back to ReAct. `generic` (empty steps) uses ReAct as before.

Plan JSON schema (excerpt):

```json
{
  "summary": "…",
  "skip_execution": false,
  "subtasks": [
    { "id": 1, "goal": "…", "done_when": "…" }
  ]
}
```

An action records the call identity, selected tool, and arguments before it reaches the runtime. The runtime applies its workspace constraint and produces one observation.

That observation becomes the reproducible record for the next decision and for later audit.
The mission passed to execution is built with `format_mission` (`Original request` / `Current subtask` / prior subtask results). Each execution loop's `TurnTrace` is **merged** at turn end so all `Action` / `Observation` remain in one trace.

## 4. Decomposing one Tool Call (for harness design)

If the execution substrate records events, this granularity is easiest to work with.

```mermaid
flowchart LR
    subgraph invoke["1 Action = 1 Invoke"]
        ID["invoke_id"]
        NAME["tool: list_dir"]
        ARGS["args: { path }"]
        POL["resolve_in_workspace"]
        RUN["ToolRuntime::execute"]
        RES["Observation: ok / output"]
    end

    ID --> NAME --> ARGS --> POL --> RUN --> RES
```

Each action gets an id so the call and its result stay paired in logs. The tool name and arguments record what was attempted; workspace checks reject out-of-bounds paths before execution; the observation records success and what the agent learned.

| Field | Meaning | Implementation |
|-------|---------|----------------|
| `invoke_id` | Key in logs · trace | `Action::invoke_id` |
| `tool` + `args` | Reproducible command | `src/tool.rs` |
| Workspace constraint | Path rejection | `resolve_in_workspace` |
| `ok` + `output` | Input to next `decide` | `Observation` |

## 5. Boundary cases (design notes)

```mermaid
flowchart TB
    A["Minimum unit = 1 Tool Call in execution phase"]
    B["Plan JSON is not an action"]
    C["Exception: batch multiple tools"]
    D["Exception: sub-agent launch"]

    A --- B
    A --- C
    A --- D

    B --> B1["PlanArtifact → policy only. No side effects"]
    C --> C1["Multiple Actions in one decide → N Invokes (current: 1 step 1 Action)"]
    D --> D1["Each two_phase execution loop ≈ child turn. Not a parent Action"]
```

Planning text is deliberately outside the action count because it changes no environment. A future batch operation would still need one recorded invocation per tool it executes.

Likewise, separate execution loops for subtasks or sub-agents are larger orchestration units. Their internal tool calls remain the minimum action units.

| Case | Treatment |
|------|-----------|
| Planning phase | Text / JSON. **Does not add Invokes** |
| Parallel tools | **N Invokes** in logs (HarnessSeed current: 1 step 1 `Action`) |
| Multiple subtasks in two_phase | **N execution loops**. Minimum unit is `Action` within each loop |
| Text only | `Thought` / `Answer` / plan summary are not actions |

The boundary stays consistent across these cases: count effects, not planning or response text. Parallelism changes how many operations occur at once, but it does not merge their audit records.

## 6. Summary

| Question | Answer |
|----------|--------|
| Minimum action unit | **1 Tool Call + Observation in the execution phase** |
| Is planning an action? | **No** (`PlanArtifact` is an intermediate artifact) |
| Can one turn have multiple Actions? | **Yes** (within one execution loop, or one loop per subtask) |
| What does the harness log? | Execution: `invoke_id`, `tool`, `args`, `Observation`. Plan: `plan` + optional `[context plan]` |

The unit is a tool call together with its recorded outcome. Plans can be logged for traceability, but they remain instructions rather than evidence that an external operation occurred.

## 7. HarnessSeed mapping (current)

| Concept | Type / API | Module |
|---------|------------|--------|
| Turn | `TurnResult` | `src/react.rs` |
| Plan artifact | `PlanArtifact`, `Subtask` | `src/plan.rs` |
| Order audit | `audit_trace` / `ArgAuditMode` | `src/tasks/audit.rs` |
| Task contract | `TaskDefinition`, `ExecStep` (`order` + `method`) | `src/tasks/spec.rs`, `tasks/*.json` — [05_task-registry.md](05_task-registry.md) |
| Planning layer loop | `run_plan_layer` + `PlanBrainMode` | `src/layer.rs`, `src/plan/brain.rs` |
| Execution step | `AgentStep` | `src/action.rs` |
| Minimum action | `Action` | `src/action.rs` |
| Observation | `Observation` | `src/action.rs` |
| In-turn trace | `TurnTrace` | `src/action.rs` (merged across subtasks when `two_phase`) |
| Subtask result | `SubtaskExecResult` | `src/react.rs` |
| Config | `react.two_phase`, `react.max_steps`, `react.use_step_driver`, `react.show_prompt` | `config/config.json`, `ReActConfig` |

The turn and plan types organize the larger lifecycle. Action, observation, and trace types preserve each concrete operation, while the task and audit modules verify that those operations satisfy a registered procedure.
With `two_phase: false` (default), one turn is **one execution loop** only. With `true`, §3 planning → serial execution applies.

The outer advance loop (phase split · `recalled` carry-over) is [07_advance-loop.md](07_advance-loop.md) (`react.advance.enabled`). REPL `SessionMemory` is thin short-term memory across turns ([09_context-memory-mapping.md §10](09_context-memory-mapping.md#10-short-term-memory-sessionmemory-implementation)).
