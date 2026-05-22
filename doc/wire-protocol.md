# ワイヤプロトコル（JSON）

CUI・GUI・別言語ホストと `ReActLoop` の間の契約。プロトコルバージョン: **1**（`WIRE_VERSION`）。

## トランスポート

- **JSON Lines**: 1 行 = 1 リクエスト、1 行 = 1 レスポンス（`cargo run -- --json`）
- **ライブラリ**: `ReActLoop::handle_wire_json` / `handle_wire_request`
- 人間向けテキスト REPL（`--json` なし）は従来どおり

## リクエスト

### `turn` — 1 ターン実行

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

`options` はすべて省略可（省略時は trace / plan / context を含める）。

### `session_clear` — 短期記憶リセット

```json
{ "type": "session_clear" }
```

### `ping` — 環境確認

```json
{ "type": "ping" }
```

## レスポンス

### `turn` 成功

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

### `turn` 失敗（ループ上限など）

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

### `protocol_error`（JSON パース失敗）

```json
{ "type": "protocol_error", "version": 1, "ok": false, "message": "..." }
```

## 関連

- 実装: `src/protocol.rs`
- ReAct 本体: [react-implementation.md](react-implementation.md)
