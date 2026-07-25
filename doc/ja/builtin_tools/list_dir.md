# list_dir

ワークスペース内のディレクトリに含まれるエントリ名を、ソート済みの改行区切りテキストで返す。

## 引数

| 名前 | 型 | 必須 | 説明 |
|------|-----|------|------|
| `path` | string | いいえ | 対象ディレクトリ（プロジェクト相対）。省略時は `.`（ルート） |

```json
{ "path": "src" }
```

## 挙動

1. `resolve_in_workspace(path)` で絶対パスに解決
2. `std::fs::read_dir` でエントリを列挙
3. 各エントリのファイル名を取得
   - ディレクトリは末尾に `/` を付与（例: `src/`）
   - ファイルは名前のみ
4. 名前を辞書順ソート
5. `\n` で連結して `output` に返す

`.` / `..` は一覧に含めない（OS の `read_dir` 結果に依存）。

## 成功時の output 例

```
Cargo.toml
README.md
src/
tests/
```

## 失敗

| 条件 | output の例 |
|------|-------------|
| パスがワークスペース外 | `path outside workspace: ...` |
| 絶対パス指定 | `absolute path not allowed: ...` |
| ディレクトリが存在しない | `list_dir failed: ...` |
| 読み取り権限なし | `list_dir failed: ...` |

## LLM からの呼び出し例

```json
{"step":"action","tool":"list_dir","args":{"path":"."}}
```

## テスト

- 統合: `LIST_FILES_USER_PROMPT`（`tests/list_files_test.rs`）

## 実装

- `src/tool.rs`: `run_list_dir`
