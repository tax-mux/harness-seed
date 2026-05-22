# read_file

ワークスペース内のテキストファイルを UTF-8 として読み取り、内容をそのまま `output` に返す。

## 引数

| 名前 | 型 | 必須 | 説明 |
|------|-----|------|------|
| `path` | string | **はい** | ファイルパス（プロジェクト相対） |

```json
{ "path": "src/main.rs" }
```

## 挙動

1. `path` 未指定 → 失敗（`read_file requires path`）
2. `resolve_in_workspace(path)` で解決（ファイルが未存在でもパス自体は解決可）
3. `std::fs::read_to_string` で全文読み込み
4. 読み取った文字列を `output` に設定

バイナリファイルは UTF-8 解釈失敗でエラーになる。部分読み・行範囲指定は未対応。

## 成功時の output 例

ファイル内容そのもの（複数行可）:

```
fn main() {
    println!("hi");
}
```

## 失敗

| 条件 | output の例 |
|------|-------------|
| `path` なし | `read_file requires path` |
| ワークスペース外 | `path outside workspace: ...` |
| ファイルなし | `read_file failed: ...` |
| ディレクトリを指定 | `read_file failed: ...`（OS エラー） |

## LLM からの呼び出し例

```json
{"step":"action","tool":"read_file","args":{"path":"config/config.json"}}
```

## 典型的な使い方

- `write_file` のあと内容確認
- 既存コードの編集前に現状把握

## テスト

- ユニット: `tool::tests::write_and_read_file_roundtrip`
- 統合: `WRITE_CODE_USER_PROMPT`（`tests/write_code_test.rs`）

## 実装

- `src/tool.rs`: `run_read_file`
