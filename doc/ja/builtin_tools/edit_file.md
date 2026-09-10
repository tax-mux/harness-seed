# edit_file

ワークスペース内のファイルで、**既存のテキストを差し替えて編集**する。`write_file` が全体を上書きするのに対し、一部だけ書き換える。リファクタやパッチ適用で、差分だけ反映したい場合に使う。

## 引数

| 名前 | 型 | 必須 | 説明 |
|------|-----|------|------|
| `path` | string | **はい** | 対象ファイル（プロジェクト相対）。`write_file` 同様に `resolve_in_workspace` 経由 |
| `old` | string | **はい** | 置き換えたい既存のテキスト |
| `new` | string | **はい** | 差し替え後のテキスト |
| `count` | number | いいえ | 置換する一致の上限（先頭から順に）。既定は `1` |
| `replace_all` | boolean | いいえ | `true` にすると一致箇所をすべて置換（`replace_all=true` を指定時は `count` 不要） |

```json
{
  "path": "src/harness/parse.rs",
  "old": "parse_old(text)",
  "new": "parse_new(text)"
}
```

## 挙動

1. `path` 未指定 → 失敗（`edit_file requires path`）
2. `resolve_in_workspace(path)` で解決（**既存ファイルのみ**。親ディレクトリ自動作成はしない）
3. ゲート検証（OpenCode 風）：
   - `old` が空 → 失敗（`edit_file requires old (text to replace)`）
   - `new` が空 → 失敗（`edit_file requires new (replacement text)`）
   - `old == new` → 失敗（`edit_file: 'old' and 'new' are identical (no change)`）
   - `file_content` / `content` のような全体テキストが渡された → 失敗し `write_file` へ誘导（`write_file is for a partial replacement. Looks like a full rewrite (found '{k}'); use write_file instead.`）
4. `text` を読み込み、`old` を `new` に差し替え
5. `count`（既定1）/ `replace_all` に応じて置換数を制御
6. `std::fs::write` で上書き。成功は「置換数 +バイト数差異 + diff 要約」を1行で返す（ファイル本文は返さない）

## 置換ロジック

- `replace_all=true`（既定外）→ 一致箇所を**すべて」差し替える
- `count=N` 指定 → **先頭から順に `N` 箇所**を差し替える
- 既定（`count` も `replace_all` も omitted）→ **最初の一つのみ**を差し替える

## 成功時の output 例

```
replaced 1 occurrence(s) in src/harness/parse.rs (16 bytes -> 18 bytes)
--- diff ---
- "parse_old(text)"
+ "parse_new(text)"
```

## 失敗

| 条件 | output の例 |
|------|-------------|
| `path` なし | `edit_file requires path` |
| ワークスペース外 / ディレクトリ | `edit_file: path is a directory (...)` / `path outside workspace: ...` |
| `old` 未指定 | `edit_file requires old (text to replace)` |
| `new` 未指定 | `edit_file requires new (replacement text)` |
| `old` == `new` | `edit_file: 'old' and 'new' are identical (no change)` |
| 全体書き込み風入力 | `edit_file is for a partial replacement. ...` |
| `old` が本文に見つからない | （一致0なので空書き込みを回避するため失敗／0件） |
| 読み込み失敗 | `edit_file failed to read: ...` |
| 書き込み失敗 | `edit_file failed to write: ...` |

## LLM からの呼び出し例

```json
{
  "step": "action",
  "tool": "edit_file",
  "args": {
    "path": "src/tool/pack.rs",
    "old": "pub const NAME: &str = \"old\";",
    "new": "pub const NAME: &str = \"edit_file\";"
  }
}
```

## 典型な使い方

1. `read_file` で編集対象を確認
2. `edit_file` で中身を差し替え（`old` は原文のサブストリング）
3. `run_cmd` で `cargo`/テスト実行
4. 再度 `read_file` で戻り値を検証

## 注意

- `tmp/` 以下への書き込みは `.gitignore` 対象（`tmp/`）か確認
- 同一テキストが複数箇所にあっても、既定は1箇所のみ置換（`count` / `replace_all` で制御）
- `old` 不一致時は空書き込みしない（`write_file` のように全体を消す事故を防ぐ）

## テスト

- ユニット: `tool::edit::tests`（`replace_first_n`, `replace_all`, `identical_edit_detected` 等）

## 実装

- `src/tool/edit.rs`: `run_edit_file` `replace_first_n` / `replace_all` / `summarize_diff`
- `src/tool/pack.rs`: `Coding` / `Full` パックに登録（`EditFileTool`）
