# コンテキスト・カラーマップ（将来拡張）

stderr の `[context map]`（v0）は **実装済み**（`src/context_map.rs`）。計測の位置づけは [../architecture/09_コンテキストマッピング.md](../architecture/09_コンテキストマッピング.md)。

## 実装済み（v0・参照）

- ターン終了時、`show_context_metrics` が ON かつ LLM 使用時に stderr へ `[context map]`
- API: `analyze_prompt_body`, `analyze_messages`, `format_colormap`

## 将来（このメモの本題）

| 項目 | 内容 |
|------|------|
| HTML / Canvas | ターン推移のヒートマップ（`logs/context.jsonl` から） |
| ステップ比較 | decide 1 → N で trace 膨張をアニメーション |
| observation 内訳 | ShellHook 適用前後の 2 列 |
| 閾値警告 | セクションが TPM 予算の X% 超で色を変える |
