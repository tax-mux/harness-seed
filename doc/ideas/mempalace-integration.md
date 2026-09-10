# mempalace 連携

HarnessSeed と **mempalace**（外部記憶・検索）を接続する。

**実装状況（2026-07）**: 実装済み（記憶 RAG + `MemoryBridge` レイヤ）。

| 部品 | 場所 |
|------|------|
| MCP / HTTP クライアント | [`adapters/mempalace-adapter`](../../adapters/mempalace-adapter/)（Cargo feature `mempalace`） |
| Bridge ラッパ | `src/memory/mempalace.rs` |
| 工場 | `src/memory/factory.rs`（`local` + `backends: ["mempalace"]`） |
| ターン注入・diary | [memory.md](../memory.md)（正本） |
| 設定 | [config/README.md](../../config/README.md) の `memory` |

本体は mempalace を **直叩きしない**。`MemoryBridge::recent_work` / `search` / `diary` のみ。

未実装（将来）: session 溢れ退避、専用ツール `memory_search`、KG 参照。

> **代替候補**: 企業文書コーパスの想起は [corpus2skill-integration.md](corpus2skill-integration.md)（Corpus2Skill）の方が適合する可能性が高い。

## 役割分担

| 層 | HarnessSeed | mempalace |
|----|-------------|-----------|
| **短期** | `TurnTrace`, `SessionMemory`（`work_log` 時のみ Previous turns） | — |
| **中期** | `recalled`（RAG が注入） | diary / search（wing 共有、room は agent） |
| **長期** | `rules` | drawer / 正本（ホスト側） |

レイアウト: `wing_{project}` 共有、`room={agent_name}` に diary 書き込み。検索は wing 全体。

## 設定例

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
  "rag": { "router": "llm", "max_queries": 3 }
}
```

詳細はアダプタ [README](../../adapters/mempalace-adapter/README.md) と [memory.md](../memory.md)。
