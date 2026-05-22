# run_cmd

ワークスペース内をカレントディレクトリとして、シェル経由で 1 コマンドを実行し、終了コードと標準出力・標準エラーを返す。

## 引数

| 名前 | 型 | 必須 | 説明 |
|------|-----|------|------|
| `command` | string | **はい** | シェルに渡すコマンド文字列全体 |
| `cwd` | string | いいえ | 作業ディレクトリ（プロジェクト相対）。省略時はプロジェクトルート |

```json
{
  "command": "cargo check",
  "cwd": "."
}
```

## 挙動

1. `command` 未指定または空白のみ → 失敗
2. `cwd` を `resolve_in_workspace` で解決（省略時は `workspace_root()`）
3. シェルでコマンド実行（起動時に `RuntimeEnvironment::detect()` で 1 回だけ検出）
   - **Windows**: `pwsh` → `powershell` → `%COMSPEC%`（通常 `cmd`）の順で試行
   - **Unix**: `$SHELL` → `bash` → `sh` の順
4. 検出結果はプロンプトの `Execution environment` にも載る
5. プロセスの stdout / stderr をキャプチャ（ブロッキング、タイムアウトなし）
6. 次の形式で `output` を組み立て

```
exit_code=<n>
--- stdout ---
<stdout 本文>--- stderr ---
<stderr 本文>
```

終了コードが取れない場合は `-1`。

## 成功時の output 例

```
exit_code=0
--- stdout ---
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.18s
--- stderr ---

```

`exit_code != 0` でも **ツール呼び出し自体は成功**（`ok: true`）。コマンド失敗は `output` 内の `exit_code` で判断する。

## 失敗

| 条件 | output の例 |
|------|-------------|
| `command` なし | `run_cmd requires command` |
| 空コマンド | `run_cmd: empty command` |
| `cwd` がワークスペース外 | `path outside workspace: ...` |
| プロセス起動失敗 | `run_cmd spawn failed: ...` |

## LLM からの呼び出し例

```json
{"step":"action","tool":"run_cmd","args":{"command":"cargo test --quiet"}}
```

Windows でも `cargo` など PATH 上のコマンドは `cmd /C` 経由で実行される。

## セキュリティ

- コマンド内容に **制限なし**（任意のシェル操作が可能）
- `cwd` はワークスペース内に限定されるが、`command` 内で絶対パスを指定すれば他場所に触れる可能性がある
- 本番・共有環境では許可コマンドリスト、タイムアウト、ユーザー権限の見直しを推奨

## 典型的な使い方

- `cargo check` / `cargo test` で書き込み後の検証
- `echo` による動作確認（ユニットテスト `run_cmd_echo`）

## 実装

- `src/tool.rs`: `run_cmd`, `run_shell_command`
