//! 組み込み `edit_file` ツール（ファイルの途中を指定して書き換える）。
//!
//! [`super::builtin::WriteFileTool`](super::builtin) が「全体を上書き」なのに対し、
//! `edit_file` は既存テキストの一部だけを差し替えます。リファクタやパッチ適用など、
//! 差分だけ反映したいユースケース向けです。

use std::path::Path;

use serde_json::Value;

use super::builtin::resolve_in_workspace;
use super::traits::{Tool, ToolContext};
use crate::action::Observation;

/// `old` と `new` が同一（実質置換ゼロ）ではないか。
fn is_identical_edit(old: &str, new: &str) -> bool {
    old == new
}

/// `replace_all` 指定で、一致箇所をすべて差し替えた結果を返す。
fn replace_all(text: &str, old: &str, new: &str) -> (String, usize) {
    if old.is_empty() {
        return (text.to_string(), 0);
    }
    let count = text.matches(old).count();
    (text.replace(old, new), count)
}

/// `count` 箇所まで（先頭から順に）`old` を `new` に差し替える。
///
/// 実装は単純な巡回置換：先頭から `` count `` 回、`old` に出るたびに差し替えていく。
/// （Rust 標準ライブラリには先頭 N 回に相当する API がないため、手動巡回。）
fn replace_first_n(text: &str, old: &str, new: &str, count: usize) -> (String, usize) {
    if old.is_empty() {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut replaced = 0usize;
    for m in text.match_indices(old) {
        let (mi, matched_old) = m;
        // 先頭から順に差し替えるため、これまでスキップした分をコピートする。
        out.push_str(&text[cursor..mi]);
        out.push_str(new);
        // 処理済みは次位置へ（累積ではなく）。先頭 N 回の置換をしたら打ち切る。
        cursor = mi + matched_old.len();
        replaced += 1;
        if replaced >= count {
            break;
        }
    }
    out.push_str(&text[cursor..]);
    (out, replaced.min(count.max(1)))
}

/// 新旧の差分（行単位の簡易 diff）を要約する。
///
/// 行を `-`(旧) / `+`(新) に分類し、先頭の共通接頭行を取り除いた差分を列げる。
/// 巨大ファイル防止のため最大 50 行差まで（超過は「... n more lines changed」で圧縮）。
fn summarize_diff(old_source: &str, new_source: &str) -> String {
    let old_lines: Vec<&str> = old_source.lines().collect();
    let new_lines: Vec<&str> = new_source.lines().collect();
    // 先頭から共通の接頭 row を数える。
    let common = old_lines
        .iter()
        .zip(new_lines.iter())
        .take_while(|(o, n)| o == n)
        .count();
    let old_slice = &old_lines[common.min(old_lines.len())..];
    let new_slice = &new_lines[common.min(new_lines.len())..];

    let diff_count = |rows: &[&str]| -> usize {
        rows.iter().filter(|r| !r.is_empty()).count().min(50)
    };
    let removed = diff_count(old_slice);
    let added = diff_count(new_slice);

    let sample = |rows: &[&str], tag: &str, n: usize| -> String {
        if rows.is_empty() {
            return String::new();
        }
        let head: Vec<&str> = rows.iter().filter(|r| !r.is_empty()).take(3).copied().collect();
        let lines: Vec<String> = head
            .iter()
            .map(|h| format!("{tag} {:?}", truncate(h, 70)))
            .collect();
        let more = n.saturating_sub(3);
        if more > 0 {
            let mut v = lines;
            v.push(format!("  ... {more} more {tag} line(s)"));
            v.join("\n")
        } else {
            lines.join("\n")
        }
    };
    let r = sample(old_slice, "-", removed);
    let a = sample(new_slice, "+", added);
    match (r.is_empty(), a.is_empty()) {
        (true, true) => String::new(),
        (false, true) => r,
        (true, false) => a,
        (false, false) => format!("{r}\n{a}"),
    }
}

/// 行を 70 文字に切り詰める（省略記号付き）。
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let cutoff: usize = s
        .chars()
        .take(max)
        .map(char::len_utf8)
        .collect::<Vec<usize>>()
        .iter()
        .sum::<usize>()
        .min(max);
    let head = &s[..cutoff];
    format!("{head}…")
}

/// 差し替え後をファイルへ書き込み、成功/失敗を返す（本文は返さない）。
fn write_and_report(
    invoke_id: u64,
    dest: &Path,
    path: &str,
    old_source: &str,
    new_source: &str,
    count: usize,
) -> Observation {
    match std::fs::write(dest, new_source) {
        Ok(()) => {
            let diff = summarize_diff(old_source, new_source);
            let mut msg = format!(
                "replaced {count} occurrence(s) in {path} ({} bytes -> {} bytes)",
                old_source.len(),
                new_source.len()
            );
            if !diff.is_empty() {
                msg.push_str(&format!("\n--- diff ---\n{diff}"));
            }
            Observation::success(invoke_id, msg)
        }
        Err(err) => {
            Observation::failure(invoke_id, format!("edit_file failed to write: {err}"))
        }
    }
}

/// edit_file ツールの実行。
///
/// 手順：パス解決 → 読み込み → 引数検証(ゲート) → 置換 → 書き込み → 差分を要約。
pub fn run_edit_file(invoke_id: u64, args: &Value, dest: &Path) -> Observation {
    let path = args.get("path").and_then(Value::as_str);
    let Some(path) = path else {
        return Observation::failure(invoke_id, "edit_file requires path");
    };
    let old = args
        .get("old")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let new = args
        .get("new")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    // 書き込み風入力: `file_content` / `content` のような全体テキストを old 無しで受けたら
    // write_file へ誘する（OpenCode 風ゲート）。
    for k in ["file_content", "filecontent", "content", "full_content", "fulltext"] {
        if let Some(full) = args.get(k).and_then(Value::as_str) {
            if !full.trim().is_empty() {
                return Observation::failure(
                    invoke_id,
                    format!(
                        "edit_file is for a partial replacement. Looks like a full rewrite (found '{k}'); use write_file instead."
                    ),
                );
            }
        }
    }

    if old.is_empty() {
        return Observation::failure(invoke_id, "edit_file requires old (text to replace)");
    }
    if new.is_empty() {
        return Observation::failure(invoke_id, "edit_file requires new (replacement text)");
    }
    if is_identical_edit(&old, &new) {
        return Observation::failure(
            invoke_id,
            "edit_file: 'old' and 'new' are identical (no change)",
        );
    }

    let text = match std::fs::read_to_string(dest) {
        Ok(t) => t,
        Err(err) => {
            return Observation::failure(invoke_id, format!("edit_file failed to read: {err}"))
        }
    };
    if old.trim().is_empty() {
        return Observation::failure(
            invoke_id,
            "edit_file: 'old' must be non-empty after trim",
        );
    }

    match (
        args.get("replace_all").and_then(Value::as_bool),
        args.get("count").and_then(Value::as_u64),
    ) {
        (Some(true), _) => {
            let (changed, count) = replace_all(&text, &old, &new);
            write_and_report(invoke_id, dest, path, &text, &changed, count)
        }
        (_r, Some(c)) => {
            let count = c as usize;
            let (changed, n) = replace_first_n(&text, &old, &new, count);
            write_and_report(invoke_id, dest, path, &text, &changed, n)
        }
        (_e, None) => {
            // 既定: 最初の一つのみ置換（OpenCode 风ゲート準拠）。
            let (changed, n) = replace_first_n(&text, &old, &new, 1);
            write_and_report(invoke_id, dest, path, &text, &changed, n)
        }
    }
}

pub struct EditFileTool;

impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn spec(&self) -> &str {
        "args: {\"path\", \"old\", \"new\", optional \"count\", optional \"replace_all\"} — replace a portion of a file (resolve_in_workspace then write)"
    }

    fn execute(&self, invoke_id: u64, args: &Value, _ctx: &ToolContext) -> Observation {
        let Some(path) = args.get("path").and_then(Value::as_str) else {
            return Observation::failure(invoke_id, "edit_file requires path");
        };
        match resolve_in_workspace(path) {
            Ok(abs) => {
                if abs.is_dir() {
                    return Observation::failure(
                        invoke_id,
                        format!("edit_file: path is a directory ({path}). Use read_file on a file path first."),
                    );
                }
                run_edit_file(invoke_id, args, &abs)
            }
            Err(err) => Observation::failure(invoke_id, err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_first_n_only_first_occurrence() {
        let (out, n) = replace_first_n("ab cd ab", "ab", "XX", 1);
        assert_eq!(n, 1);
        assert_eq!(out, "XX cd ab");
    }

    #[test]
    fn replace_first_n_up_to_count() {
        let (out, n) = replace_first_n("ab cd ab ef ab", "ab", "XX", 2);
        assert_eq!(n, 2);
        assert_eq!(out, "XX cd XX ef ab");
    }

    #[test]
    fn replace_all_replaces_everything() {
        let (out, n) = replace_all("ab ab ab", "ab", "XX");
        assert_eq!(n, 3);
        assert_eq!(out, "XX XX XX");
    }

    #[test]
    fn identical_edit_detected() {
        assert!(is_identical_edit("a", "a"));
        assert!(!is_identical_edit("a", "b"));
    }

    #[test]
    fn summarize_diff_shows_removed_added() {
        let diff = summarize_diff("line0\nold\n", "line0\nnew\n");
        assert!(diff.contains("- \"old\""), "diff: {diff}");
        assert!(diff.contains("+ \"new\""), "diff: {diff}");
    }
}
