# ツールプラグインとパッケージング

HarnessSeed の ReAct ツールは **in-process プラグイン**として登録する。次の段階で「基本セット」「コーディング拡張」などの **ツールパック**を設定で切り替える。

## 構成

| モジュール | 役割 |
|-----------|------|
| `src/tool/traits.rs` | `Tool` trait、`ToolContext` |
| `src/tool/registry.rs` | `ToolRegistry` — 登録・実行・カタログ |
| `src/tool/pack.rs` | `ToolPack` — 束ね登録 |
| `src/tool/builtin.rs` | 組み込み実装 |
| `src/tool/mod.rs` | `ToolRuntime` — invoke_id + レジストリ |

## ツールパック

| パック ID | ツール |
|-----------|--------|
| `basic` | `echo`, `time` |
| `coding` | `list_dir`, `grep`, `read_file`, `write_file`, `run_cmd` |
| `web_search` | `web_search`（Brave API キー必須） |
| `full` | 上記すべて（キーあり時に web 含む） |

## 設定

`config.json` の `tools.packs`（スイッチ形式）:

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

- **未設定 / `{}`**: `basic` + `coding`。Brave キーがあれば `web_search` を自動追加。
- **明示スイッチ**: `true` のパックのみ有効。`web_search: false` で自動追加も抑止。
- **`full: true`**: 全パック一括（Brave キーあり時は web 含む）。
- 旧形式 `["basic", "coding"]` も後方互換で読める。

## プロンプト連携

- `ToolRuntime::catalog()` → `PromptBlocks.tool_catalog`
- `TurnPromptContext` の system に動的カタログを載せる（固定 `tools_catalog()` は廃止）
- `web_search_enabled` はレジストリに `web_search` が登録されているかで決まる

## ホストからの拡張

```rust
let mut rt = ToolRuntime::with_packs(env, brave, &[ToolPack::Basic, ToolPack::Coding]);
rt.register_plugin(Box::new(MyCustomTool));
blocks.tool_catalog = rt.catalog();
```

## 次の段階（案）

- **パックの外部定義**: `packs/my-pack.toml` + dylib / WASM（未実装）
- **タスク契約との整合**: `tasks/*.json` の `steps[].tool` が未登録ツールなら計画層で警告
- **MCP ブリッジ**: 別プロセスツールを `Tool` 実装でラップ
