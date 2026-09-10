# time

現在時刻を Unix エポック秒（UTC 基準の `SystemTime`）で返す。

## 引数

なし（`{}`）。

```json
{}
```

## 挙動

1. `SystemTime::now()` を取得
2. `duration_since(UNIX_EPOCH)` の秒数を整数化（失敗時は `0`）
3. `unix_epoch_secs=<n>` 形式の 1 行を `output` に返す

タイムゾーン変換や ISO8601 文字列は行わない。

## 成功時の output 例

```
unix_epoch_secs=1779373405
```

## 失敗

通常は失敗しない。クロック取得に失敗した場合のみ秒が `0` になる。

## LLM からの呼び出し例

```json
{"step":"action","tool":"time","args":{}}
```

## 実装

- `src/tool.rs`: `run_time`
- ルール頭脳: 入力 `time`（大文字小文字無視）で `time_action` を発行
