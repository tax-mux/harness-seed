# write_file

ワークスペース内のファイルにテキストを書き込む。親ディレクトリが無ければ自動作成する。

## 引数

| 名前 | 型 | 必須 | 説明 |
|------|-----|------|------|
| `path` | string | **はい** | 書き込み先（プロジェクト相対） |
| `content` | string | いいえ | ファイル全文。省略時は空ファイル |

```json
{
  "path": "tmp/agent_hello.rs",
  "content": "fn main() { println!(\"hi\"); }\n"
}
```

## 挙動

1. `path` 未指定 → 失敗（`write_file requires path`）
2. `resolve_in_workspace(path)` で解決（**未作成ファイルでも可**）
3. 親ディレクトリに `std::fs::create_dir_all`
4. `std::fs::write` で `content` をバイト列として上書き書き込み
5. 成功時はバイト数と論理パスを要約した 1 行を返す（ファイル内容は返さない）

追記（append）モードはない。常に全体置換。

## 成功時の output 例

```
wrote 35 bytes to tmp/agent_hello.rs
```

## 失敗

| 条件 | output の例 |
|------|-------------|
| `path` なし | `write_file requires path` |
| ワークスペース外 | `path outside workspace: ...` |
| ディレクトリ作成失敗 | `write_file mkdir failed: ...` |
| 書き込み失敗 | `write_file failed: ...` |

## LLM からの呼び出し例

```json
{
  "step": "action",
  "tool": "write_file",
  "args": {
    "path": "tmp/example.rs",
    "content": "fn main() {}\n"
  }
}
```

## 典型的な使い方

1. `write_file` で新規・更新
2. `read_file` で確認
3. `run_cmd` で `cargo check` など

## 注意

- `tmp/` 以下への書き込みは `.gitignore` 対象（`tmp/`）
- 既存ファイルは警告なしで上書き

## テスト

- ユニット: `tool::tests::write_and_read_file_roundtrip`
- 統合: `WRITE_CODE_USER_PROMPT`（`tests/write_code_test.rs`）

## 実装

- `src/tool.rs`: `run_write_file`
