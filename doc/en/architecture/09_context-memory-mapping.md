# Context memory mapping

A diagrammatic view of **one Chat Completions request** sent to the LLM (in HarnessSeed: `TurnPromptContext::render`), organized by **purpose, memory layer, and storage location**.

- ReAct implementation: [08_react-implementation.md](08_react-implementation.md)
- Minimum action unit: [10_agent-minimum-action-unit.md](10_agent-minimum-action-unit.md)
- Built-in tools: [../builtin_tools/README.md](../builtin_tools/README.md)
- Japanese version: [09_コンテキストマッピング.md](../../ja/architecture/09_コンテキストマッピング.md)

**Status (as of 2026-07)**

| Area | Status |
|------|--------|
| In-turn `TurnTrace` | Implemented |
| Session short-term `SessionMemory` → `Previous turns` | **Implemented** (§10). Injected **only on the work-log path (`work_log`)** |
| Context metrics / `logs/context.jsonl` | Implemented (Observation has a character cap) |
| External memory (search / diary) | **Implemented** — [03_memory-layer.md](03_memory-layer.md) (memory RAG + `MemoryBridge`) |
| Rules file injection (`prompt.rules_paths`) | **Implemented** (`PromptBlocks`) |
| trace / session summarization | Not implemented |

---

## 1. One-page view: contents of the context window

```mermaid
flowchart TB
    subgraph ctx["Context carried in one LLM call"]
        direction TB
        SYS["system block"]
        USR["user block (multiple sections)"]
    end

    subgraph sys_parts["system purposes"]
        R["Behavior rules · output format"]
        T["Tool definitions · catalog"]
        L["Long-term reference (optional) rules / SSoT excerpts"]
    end

    subgraph usr_parts["user purposes"]
        M["Mid/long-term recall (optional) external search"]
        P["Short-term: Previous turns ✓"]
        G["Current goal User input"]
        TR["Short-term: Turn trace so far"]
        NX["Instruction Next step JSON, etc."]
    end

    SYS --> R
    SYS --> T
    SYS --> L
    USR --> M
    USR --> P
    USR --> G
    USR --> TR
    USR --> NX

    sys_parts -.-> SYS
    usr_parts -.-> USR
```

**Principles**

| Layer | Primary placement | Change frequency |
|-------|-------------------|------------------|
| Near-invariant policy | **system** | Low |
| Current request · in-progress work | **user** (block split) | High |

---

## 2. Memory layer × storage location

```mermaid
flowchart LR
    subgraph short["Short-term"]
        ST1["TurnTrace\n(in-turn)"]
        ST2["SessionMemory\n(REPL session)"]
        ST3["On-prompt\nPrevious turns / trace"]
    end

    subgraph mid["Mid-term"]
        MD1["Memory RAG\nwork_log / knowledge"]
        MD2["MemoryBridge\nlocal + backends"]
    end

    subgraph long["Long-term"]
        LG1["Canonical documents\nrules / SSoT"]
        LG2["External KG\n/ drawer"]
    end

    subgraph proc["Inside HarnessSeed process"]
        ST1
        ST2
    end

    subgraph ext["External store"]
        MD1
        LG1
        LG2
    end

    MD1 -.->|search at turn start → user| ST3
    LG1 -.->|load rules at startup → system or user| ST3
    ST1 --> ST3
    ST2 --> ST3
```

| Memory layer | Role | Storage (recommended) | HarnessSeed today |
|--------------|------|----------------------|-------------------|
| **Short-term** | Current turn · recent utterances | Heap `TurnTrace` / `SessionMemory` → **user block** | trace + **SessionMemory** (`Previous turns`, reset on REPL `clear`) |
| **Mid-term** | Days–weeks of work · history | External diary, search | Not connected |
| **Long-term** | Conventions · canonical sources · relations | Canonical documents / external KG | Not connected |

---

## 3. Order within the user block (recommended layout)

Top to bottom: **stable → request → latest facts** reads best.

```mermaid
block-beta
    columns 1

    block:recall:2
        columns 1
        recallTitle["Recalled context (optional)"]
        memSearch["External search results"]
        docRecall["Canonical / rules excerpts"]
    end

    block:session:2
        columns 1
        sessTitle["Session memory (optional)"]
        prev["Previous turns: last N turns ✓"]
    end

    block:task:1
        columns 1
        goal["User input: current goal"]
    end

    block:work:2
        columns 1
        traceTitle["Working memory (in-turn)"]
        trace["Turn trace so far\nthought / action / observation"]
    end

    block:cue:1
        columns 1
        next["Next step JSON:"]
```

| Order | Section | Purpose | Typical source |
|-------|---------|---------|----------------|
| 1 | Recalled context | Quotes from long/mid-term | External store, rules files |
| 2 | Previous turns | Short-term (conversation continuity) | `SessionMemory` |
| 3 | User input | **Current task** | REPL one-line input |
| 4 | Turn trace so far | **In-progress facts** | `TurnTrace` |
| 5 | Next step JSON | Output-format reminder | Harness fixed text |

HarnessSeed **current** (`src/llm/brain.rs`): **2 + 3 + 4 + 5** (`Previous turns` present. 1 · external memory / rules injection not connected).

---

## 4. Purposes within the system block

```mermaid
flowchart TB
    SYS["ChatMessage::system"]

    SYS --> A["ReAct rules\nJSON single object only"]
    SYS --> B["Schema\nthought / action / answer"]
    SYS --> C["Available tools list"]
    SYS --> D["Tool catalog\n(tools_catalog dynamic)"]
    SYS --> E["Long-term policy (optional)\nrules / SSoT excerpts"]

    style E stroke-dasharray: 5 5
```

| Content | Placement | Update frequency |
|---------|-----------|------------------|
| Output format · prohibitions | Fixed in system | On code change |
| Tool names · argument summary | system + catalog | On tool add |
| Project constitution · rules | system head or tail (optional) | On canonical document update |

**Do not put the current goal in system** (use user `User input` instead).

---

## 5. Lifecycle: when each piece is loaded

```mermaid
sequenceDiagram
    participant U as User
    participant H as HarnessSeed
    participant ST as Short-term memory
    participant MP as External mid-term memory
    participant DOC as Canonical documents
    participant M as LLM

    Note over H,DOC: Startup (optional)
    H->>DOC: load rules / SSoT
    DOC-->>H: long-term block

    U->>H: REPL input (turn start)
    H->>MP: search (optional)
    MP-->>H: mid-term block
    H->>ST: read Previous turns from SessionMemory

    loop each decide (within one turn)
        H->>M: system + user (Previous turns + trace)
        M-->>H: thought / action / answer
        H->>ST: append to TurnTrace
    end

    H->>ST: SessionMemory.push_turn (user + answer)
    H->>MP: diary_write (optional)
    H-->>U: Answer
```

| Timing | Short-term | Mid-term | Long-term |
|--------|------------|----------|-----------|
| Process startup | — | — | Load rules / canonical (optional · not yet) |
| Turn start | **Implemented**: `SessionMemory` → `Previous turns` at user head | search → user (not yet) | Load (not yet) |
| Each `decide` | `TurnTrace` grows (re-sent only within turn) | — | — |
| Turn end | **Implemented**: `push_turn(user, answer)` | diary_write (not yet) | Usually not written |
| REPL `clear` | **Implemented**: `SessionMemory::clear()` | — | — |

---

## 6. When context is long (forgetting)

```mermaid
flowchart LR
    subgraph window["Context window (bounded)"]
        direction LR
        S["system"]
        OLD["Older user / trace"]
        NEW["Newer trace"]
        OUT["Generation headroom"]
    end

    S --- OLD --- NEW --- OUT

    OLD -.->|drops when overflow| X["Effective forgetting"]
    NEW --> M["Easier for model to attend"]

    MP["Evacuate to external store"] -.->|before turn end| OLD
```

| Phenomenon | Cause | Mitigation |
|------------|-------|------------|
| Old observations stop working in-turn | trace grows linearly | Summarize trace · keep last N steps only |
| REPL forgets earlier utterances (beyond K) | Turns older than `session_max_turns` are dropped | Tune K, `clear`, future: evacuate to external store |
| Complete loss of overflowed content | Outside window | Evacuate to external diary / local summary file |

Check metrics via `[context step]` / `prompt_tokens` in `logs/context.jsonl`.

---

## 7. Quick reference: purpose × placement

| Purpose | Placement | Memory layer | Recommended store |
|---------|-----------|--------------|-------------------|
| JSON output format only | system | — | Code |
| Tool list | system + catalog | — | Code + [../builtin_tools/README.md](../builtin_tools/README.md) |
| Project rules / conduct conventions | system (excerpt) | Long-term | Rules files / canonical |
| Project spec · decisions | user head or system | Long-term | Canonical documents |
| Recall similar past work | user head | Mid-term | External search |
| Recent conversation | user `Previous turns` | Short-term | In-process SessionMemory |
| Current request | user `User input` | Short-term | Input itself |
| Thought / Action / Observation | user `Turn trace` | Short-term | TurnTrace |
| Raw tool output | observation in trace | Short-term | TurnTrace |
| Cross-session diary | (injection or tool) | Mid-term | External diary |
| Entity relations | user excerpt or tool | Long-term | External KG |

---

## 8. Mapping to HarnessSeed today

```mermaid
flowchart TB
    subgraph impl["Implemented"]
        I1["SYSTEM_PROMPT + Tool catalog → system"]
        I2["Previous turns + User input + Turn trace → user"]
        I3["TurnTrace / context_usages"]
        I4["SessionMemory (REPL session)"]
        I5["logs/context.jsonl"]
    end

    subgraph planned["Recommended · not implemented"]
        P1["External memory bridge"]
        P2["trace / session summarization"]
    end

    I1 --> SYS["system"]
    I2 --> USR["user"]
    I3 --> USR
    I4 --> I2
    I5 -.->|metrics log; not inference input| LOG["File"]
```

| Mapping element | Source |
|-----------------|--------|
| system rules · tools | `src/llm/brain.rs` `SYSTEM_PROMPT` + `tools_catalog()` |
| user `Previous turns` | `src/session.rs` `format_for_prompt` ← `ReActLoop.session` |
| user goal + trace | `src/llm/brain.rs` `build_messages` |
| Short-term in-turn | `src/action.rs` `TurnTrace` |
| Short-term cross-session | `src/session.rs` `SessionMemory` / `src/react.rs` |
| Metrics | `src/context_metrics.rs`, `src/context_log.rs` |

---

## 10. Short-term memory (SessionMemory) implementation

Keeps **completed turn history** valid only during a REPL session and loads it into later LLM calls as `Previous turns:`.

### 10.1 Data structures

```mermaid
flowchart LR
    RL["ReActLoop"]
    SM["SessionMemory"]
    PT["PastTurn × N"]
    RL -->|owns| SM
    SM --> PT
    PT --> U["user_input"]
    PT --> A["answer (final response only)"]
```

| Type | File | Contents |
|------|------|----------|
| `PastTurn` | `src/session.rs` | One turn: `user_input` + `answer` |
| `SessionMemory` | `src/session.rs` | `Vec<PastTurn>` + limit settings |

**Intentionally not stored**

- In-turn `thought` / `action` / `observation` (`TurnTrace` is discarded at turn end)
- Full tool output (summarization if needed: future §10.4)

### 10.2 Prompt placement

User block layout in `LlmBrain::build_messages` (`src/llm/brain.rs`):

```
{Previous turns: (omitted if empty)}

User input:
{current REPL one-liner}

Turn trace so far:
{trace for this turn}

Next step JSON:
```

`Previous turns` format (`SessionMemory::format_for_prompt`):

```text
Previous turns:
[turn 1]
User: ...
Assistant: ...
[turn 2]
...
```

### 10.3 Lifecycle (implementation)

```mermaid
sequenceDiagram
    participant R as ReActLoop
    participant S as SessionMemory
    participant B as AgentBrain
    participant L as LLM

    Note over R,S: At turn start, S holds only previously completed turns

    loop max_steps
        R->>B: decide(TurnPromptContext)
        B->>L: messages (blocks + S → Previous turns)
        L-->>B: thought / action / answer
        R->>R: update TurnTrace
    end

    R->>S: push_turn(input, answer)
    Note over S: overflow removed from front via remove(0)
```

| Operation | Timing | Code |
|-----------|--------|------|
| Read | Each `decide` | `TurnPromptContext { blocks, input, trace, session }` |
| Write | Right after `Answer` is fixed | `session.push_turn(user_input, answer)` |
| Reset | REPL `clear` / `forget` / `reset` | `session.clear()` (`src/react.rs` `run_repl`) |

The third argument `session` to `AgentBrain::decide` is **accepted by all brains**, but only **LlmBrain** reflects it in the prompt today (`SimpleRuleBrain` does not use it).

### 10.4 Limits and configuration

| Item | Default | Setting |
|------|---------|---------|
| Turns retained | 8 | `react.session_max_turns` (`config/config.json`) |
| Max chars per field | 2000 | Code constant `SessionMemory::DEFAULT_MAX_CHARS_PER_FIELD` (overflow truncated with `…`) |

`ReActConfig::session_max_turns` ← resolved via `AppConfig::react_config()`. `SessionMemory::new(...)` is created in `ReActLoop::new`.

### 10.5 Observation · verification

- **REPL**: From turn 2 onward in the same session, ask about something said earlier
- **Log**: Each step in `logs/context.jsonl` includes `Previous turns` in `prompt` (from turn 2 onward)
- **stderr**: `[context step]` `prompt_tokens` tends to grow as turns progress

### 10.6 Not implemented (next steps for short-term)

| Item | Description |
|------|-------------|
| trace summarization | Token control when one turn has many steps |
| session summarization | Compress meaning beyond N turns to shorten Previous turns |
| Persistence | `SessionMemory` is lost on process exit (file / external store not wired) |
| Rule brain | `SimpleRuleBrain` responses that reference session (demo · optional) |

---

## 11. Related documents

| Document | Contents |
|----------|----------|
| [08_react-implementation.md](08_react-implementation.md) | ReAct loop · brains · logging |
| [10_agent-minimum-action-unit.md](10_agent-minimum-action-unit.md) | Action = 1 Tool Call |
| [../builtin_tools/README.md](../builtin_tools/README.md) | Tool specifications |
| [../../config/README.md](../../../config/README.md) | Runtime config (includes `session_max_turns`) |
