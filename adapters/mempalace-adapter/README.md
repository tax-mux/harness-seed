# mempalace-adapter

HarnessSeed から **疎結合**で使う mempalace クライアント。

## 既定: MCP stdio（Cursor と同じ）

[`mcp.json`](../../../../.cursor/mcp.json) と同様に子プロセスを起動します:

```text
python -m mempalace.mcp_server
```

改行区切り JSON-RPC で `initialize` → `tools/call`（`mempalace_search` / `diary_read` / `diary_write`）。

```json
"memory": {
  "local": true,
  "backends": ["mempalace"],
  "providers": {
    "mempalace": {
      "protocol": "mcp_stdio",
      "command": "C:\\Python312\\python.exe",
      "args": ["-m", "mempalace.mcp_server"],
      "agent_name": "harness-seed"
    }
  }
}
```

`command` / `args` を省略すると `python`（Windows では `C:\Python312\python.exe` があればそれ）と `["-m","mempalace.mcp_server"]` を使います。

## その他プロトコル

| `protocol` | 用途 |
|------------|------|
| `mcp_stdio`（既定） | 上記 MCP 子プロセス |
| `tools_path` | `POST {base_url}/tools/{name}` |
| `mcp_jsonrpc` | `POST {base_url}` に `tools/call` |

## レイヤ

local diary の**置き換えではない**。`backends` に足すだけ。MCP 不通時は mempalace レイヤだけ失敗し、local は残ります。

## テスト

```bash
cargo test -p mempalace-adapter
# 実 MCP（python + mempalace が必要）:
cargo test -p mempalace-adapter mcp_stdio_live -- --ignored --nocapture
```
