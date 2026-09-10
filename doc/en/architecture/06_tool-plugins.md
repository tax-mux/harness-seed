# Tool Plugins and Packaging

## What this is

How execution-layer tools register as **in-process plugins** and are enabled in groups via pack names in config.

Glossary: [glossary.md](glossary.md)

## When to use / not use

- Use: change the built-in tool set or add a custom tool
- Skip: default packs are enough and you only need per-tool args → [builtin_tools](../builtin_tools/README.md)

## Plain flow

Pack config → register `Tool`s → catalog appears in prompts / execution

HarnessSeed ReAct tools register as **in-process plugins**. Built-ins are enabled via **tool packs** (`tools.packs`).

Per-tool args and failure rules: [builtin_tools/README.md](../builtin_tools/README.md)

Japanese version: [06_ツールプラグイン.md](../../ja/architecture/06_ツールプラグイン.md)

## Layout

| Module | Role |
|--------|------|
| `src/tool/traits.rs` | `Tool` trait, `ToolContext` |
| `src/tool/registry.rs` | `ToolRegistry` — register / execute / catalog |
| `src/tool/pack.rs` | `ToolPack` — bundled registration |
| `src/tool/builtin.rs` | Built-in implementations |
| `src/tool/mod.rs` | `ToolRuntime` — invoke_id + registry |

## Tool packs

| Pack ID | Tools |
|---------|-------|
| `basic` | `echo`, `time` |
| `coding` | `list_dir`, `grep`, `read_file`, `write_file`, `run_cmd` |
| `web_search` | `web_search` (Brave API key required) |
| `full` | All of the above (web included when key present) |

## Config

`tools.packs` in `config.json` (switch form):

```json
{
  "tools": {
    "packs": {
      "basic": true,
      "coding": true,
      "web_search": false
    },
    "brave_search": { "api_key": "..." }
  }
}
```

- **Unset / `{}`**: `basic` + `coding`. Auto-add `web_search` if Brave key exists.
- **Explicit switches**: only packs set `true`. `web_search: false` also blocks auto-add.
- **`full: true`**: enable all packs (web when key present).
- Legacy `["basic", "coding"]` still parses.

Details: [config/README.md](../../../config/README.md)

## Prompt wiring

- `ToolRuntime::catalog()` → `PromptBlocks.tool_catalog`
- Dynamic catalog injected into `TurnPromptContext` system
- `web_search_enabled` follows whether `web_search` is registered

## Host extension

```rust
let mut rt = ToolRuntime::with_packs(env, brave, &[ToolPack::Basic, ToolPack::Coding]);
rt.register_plugin(Box::new(MyCustomTool));
blocks.tool_catalog = rt.catalog();
```

If a task’s required `method` is missing from the registry, plan resolve demotes it to freeform ([05_task-registry.md](05_task-registry.md)).

## Not implemented (ideas)

| Item | Note |
|------|------|
| External pack defs (toml / dylib / WASM) | Future |
| MCP → `Tool` wrap | Intended with [ideas/tool-attention-reuse-ideas.md](../../ja/ideas/tool-attention-reuse-ideas.md) |

## Related

- [02-01_tool-selection.md](02-01_tool-selection.md)
- [05_task-registry.md](05_task-registry.md)
