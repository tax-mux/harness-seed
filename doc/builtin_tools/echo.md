# echo

メッセージ文字列を変更せず `Observation` に返す。デバッグ・記録用の最小ツール。

## 引数

| 名前 | 型 | 必須 | 説明 |
|------|-----|------|------|
| `message` | string | いいえ | 返すテキスト。省略時は空文字 |

```json
{ "message": "hello" }
```

## 挙動

1. `args.message` を文字列として取得（無ければ `""`）
2. `Observation::success` でその文字列を `output` に設定

副作用なし。ファイル・シェルには触れない。

## 成功時の output 例

```
hello
```

## 失敗

引数不正では失敗しない（`message` 非文字列の場合は `unwrap_or("")` 相当で空扱いにはならず、`as_str` で `None` → 空文字）。

## LLM からの呼び出し例

```json
{"step":"action","tool":"echo","args":{"message":"ping"}}
```

## 実装

- `src/tool.rs`: `run_echo`
- ルール頭脳 `SimpleRuleBrain` も `echo <text>` 入力で同ツールを利用
