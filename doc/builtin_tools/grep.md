# grep

ワークスペース内のテキストファイルを再帰的に検索し、マッチ行を `path:line:content` 形式で返す。

## 引数

| 名前 | 型 | 必須 | 説明 |
|------|-----|------|------|
| `pattern` | string | **はい** | 検索文字列（部分一致。正規表現は未対応） |
| `path` | string | いいえ | 起点（ファイルまたはディレクトリ）。省略時は `.` |
| `ignore_case` | bool | いいえ | 大文字小文字を無視（ASCII） |
| `glob` | string | いいえ | 簡易フィルタ（例: `*.rs`） |
| `max_results` | number | いいえ | 最大マッチ行数（既定 200、上限 2000） |

```json
{
  "pattern": "ReActLoop",
  "path": "src",
  "glob": "*.rs",
  "ignore_case": false
}
```

## 挙動

1. `pattern` 未指定または空 → 失敗
2. `resolve_in_workspace(path)` で解決
3. ディレクトリの場合は再帰走査（`.git`, `target`, `node_modules` 等はスキップ）
4. 1 ファイルあたり 1 MiB 超はスキップ。UTF-8 でない／バイナリはスキップ
5. マッチ行を `相対path:行番号:行内容` で列挙
6. 末尾にサマリ行（件数・走査ファイル数）

## 成功時の output 例

```
src/react.rs:97:pub struct ReActLoop<E: AgentBrain> {

---
1 match line(s) in 1 file(s)
```

マッチなし:

```
no matches for 'not_found_xyz' (42 file(s) searched)
```

## 失敗

| 条件 | output の例 |
|------|-------------|
| `pattern` なし | `grep requires pattern` |
| パスがワークスペース外 | `path outside workspace: ...` |
| パス不存在 | `grep path not found: ...` |

## LLM からの呼び出し例

```json
{"step":"action","tool":"grep","args":{"pattern":"run_turn","path":"src","glob":"*.rs"}}
```

## 実装

- `src/grep.rs`: 走査・マッチ
- `src/tool.rs`: `run_grep`

## テスト

- ユニット: `grep::tests`
- 統合: `tool::tests::grep_finds_in_src`
