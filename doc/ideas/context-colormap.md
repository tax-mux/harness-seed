# コンテキスト・カラーマップ（一部実装済み）

プロンプト内の**どのブロックが太いか**を一目で見る。信用・ブランド（限界内で完了）のための可視化。

## 実装済み（v0）

- モジュール: `src/context_map.rs`
- ターン終了時、`show_context_metrics` が ON かつ LLM 使用時に stderr へ `[context map]`
- セクション: `react_core`, `tool_catalog`, `rules`, `recalled`, `previous_turns`, `user_input`, `turn_trace`, `next_step` など
- ANSI カラーバー（`█` / `░`）+ 推定 tok / 割合 %

```text
[context map]
prompt sections: 4200 chars / ~1050 tok
react_core      ████████████░░░░░░░░░░░░   520 tok  49.5%
turn_trace      ██████░░░░░░░░░░░░░░░░░░   280 tok  26.7%
...
```

API: `analyze_prompt_body`, `analyze_messages`, `format_colormap`（`harness_seed` で公開）

## 将来（アイディア）

| 項目 | 内容 |
|------|------|
| HTML / Canvas | ターン推移のヒートマップ（`logs/context.jsonl` から） |
| ステップ比較 | decide 1 → N で trace 膨張をアニメーション |
| observation 内訳 | ShellHook 適用前後の 2 列 |
| 閾値警告 | セクションが TPM 予算の X% 超で色を変える |

正本 doc（`context-memory-mapping.md`）には具体機能名を書かず、計測・マッピングのみ記載。
