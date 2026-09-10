# Wire Protocol (JSON)

Contract between CUI / GUI / other-language hosts and `ReActLoop`. Protocol version: **1** (`WIRE_VERSION`).

Japanese version: [11_ワイヤプロトコル.md](../../ja/architecture/11_ワイヤプロトコル.md)

## Transport

- **JSON Lines**: one line = one request, one line = one response (`cargo run -- --json`)
- **Library**: `ReActLoop::handle_wire_json` / `handle_wire_request`
- Human text REPL (without `--json`) unchanged

## Request

### `turn` — run one turn

```json
{
  "type": "turn",
  "user_input": "list files in src",
  "options": {
    "include_trace": true,
    "include_plan": true,
    "include_context": true,
    "max_observation_chars": 8000
  }
}
```

All `options` are optional (default: include trace / plan / context).

### `session_clear` — reset short-term memory

```json
{ "type": "session_clear" }
```

### `ping` — environment check

```json
{ "type": "ping" }
```

## Response

### `turn` success

```json
{
  "type": "turn",
  "version": 1,
  "ok": true,
  "answer": "...",
  "steps_used": 3,
  "session_turns": 1,
  "trace": { "thoughts": [], "actions": [], "observations": [] },
  "plan": null,
  "subtask_results": [],
  "context": { "llm_calls": 2, "prompt_tokens": 1200, "token_source": "estimated" }
}
```

### `turn` failure (e.g. step cap)

```json
{
  "type": "turn",
  "version": 1,
  "ok": false,
  "answer": "",
  "steps_used": 0,
  "session_turns": 0,
  "error": { "code": "max_steps_exceeded", "message": "..." }
}
```

### `session_clear`

```json
{ "type": "session_clear", "version": 1, "ok": true, "session_turns": 0 }
```

### `ping`

```json
{
  "type": "ping",
  "version": 1,
  "runtime": { "os": "windows", "arch": "x86_64", "shell_label": "...", "shell_program": "pwsh" },
  "harness_version": "0.1.0"
}
```

### `protocol_error` (JSON parse failure)

```json
{ "type": "protocol_error", "version": 1, "ok": false, "message": "..." }
```

## Related

- Implementation: `src/protocol.rs`
- ReAct core: [08_react-implementation.md](08_react-implementation.md)
