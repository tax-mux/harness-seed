//! ワークスペース内のテキスト検索（組み込み `grep` ツール）。

use std::path::{Path, PathBuf};

const SKIP_DIR_NAMES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".cursor",
    "dist",
    "build",
];

const MAX_FILE_BYTES: u64 = 1_024 * 1024;

/// `grep` ツールのオプション。
#[derive(Debug, Clone)]
pub struct GrepOptions {
    pub pattern: String,
    pub ignore_case: bool,
    pub max_results: usize,
    /// 例: `"*.rs"` — `*` 接頭の拡張子フィルタのみ（簡易）。
    pub glob: Option<String>,
}

/// ワークスペース `root` 内の `search_path` 以下を検索する。
pub fn grep_in_workspace(
    root: &Path,
    search_path: &Path,
    opts: &GrepOptions,
) -> Result<String, String> {
    if opts.pattern.is_empty() {
        return Err("grep requires non-empty pattern".into());
    }

    let root = root
        .canonicalize()
        .map_err(|e| format!("workspace root invalid: {e}"))?;
    let search_path = if search_path.exists() {
        search_path
            .canonicalize()
            .map_err(|e| format!("grep path invalid: {e}"))?
    } else {
        return Err(format!("grep path not found: {}", search_path.display()));
    };

    if !search_path.starts_with(&root) {
        return Err("path outside workspace".into());
    }

    let mut matches = Vec::new();
    let mut files_searched = 0u32;

    if search_path.is_file() {
        search_file(&root, &search_path, opts, &mut matches, &mut files_searched);
    } else if search_path.is_dir() {
        walk_dir(&root, &search_path, opts, &mut matches, &mut files_searched)?;
    } else {
        return Err(format!("not a file or directory: {}", search_path.display()));
    }

    if matches.is_empty() {
        return Ok(format!(
            "no matches for '{}' ({} file(s) searched)",
            opts.pattern, files_searched
        ));
    }

    let truncated = matches.len() >= opts.max_results;
    let mut out = matches.join("\n");
    out.push_str(&format!(
        "\n\n---\n{} match line(s) in {} file(s){}",
        matches.len(),
        files_searched,
        if truncated {
            format!(" (limit {})", opts.max_results)
        } else {
            String::new()
        }
    ));
    Ok(out)
}

fn walk_dir(
    root: &Path,
    dir: &Path,
    opts: &GrepOptions,
    matches: &mut Vec<String>,
    files_searched: &mut u32,
) -> Result<(), String> {
    if matches.len() >= opts.max_results {
        return Ok(());
    }

    let entries = std::fs::read_dir(dir).map_err(|e| format!("grep read_dir failed: {e}"))?;
    let mut names: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    names.sort();

    for path in names {
        if matches.len() >= opts.max_results {
            break;
        }
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if SKIP_DIR_NAMES.contains(&file_name) {
                continue;
            }
            walk_dir(root, &path, opts, matches, files_searched)?;
        } else if path.is_file() {
            search_file(root, &path, opts, matches, files_searched);
        }
    }
    Ok(())
}

fn search_file(
    root: &Path,
    path: &Path,
    opts: &GrepOptions,
    matches: &mut Vec<String>,
    files_searched: &mut u32,
) {
    if matches.len() >= opts.max_results {
        return;
    }

    let rel = rel_path(root, path);
    if let Some(ref glob) = opts.glob {
        if !path_matches_glob(&rel, glob) {
            return;
        }
    }

    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return,
    };
    if meta.len() > MAX_FILE_BYTES {
        return;
    }

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return,
    };
    if bytes.contains(&0) {
        return;
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => t,
        Err(_) => return,
    };

    *files_searched += 1;
    for (line_no, line) in text.lines().enumerate() {
        if matches.len() >= opts.max_results {
            break;
        }
        if line_matches(line, &opts.pattern, opts.ignore_case) {
            matches.push(format!("{}:{}:{}", rel, line_no + 1, line));
        }
    }
}

fn line_matches(line: &str, pattern: &str, ignore_case: bool) -> bool {
    if ignore_case {
        let line_lower = line.to_ascii_lowercase();
        let pat_lower = pattern.to_ascii_lowercase();
        line_lower.contains(&pat_lower)
    } else {
        line.contains(pattern)
    }
}

fn path_matches_glob(rel: &str, glob: &str) -> bool {
    let glob = glob.trim();
    if let Some(suffix) = glob.strip_prefix('*') {
        rel.ends_with(suffix)
    } else {
        rel.ends_with(glob) || rel.split('/').any(|seg| seg == glob)
    }
}

fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_pattern_in_file() {
        let root = std::env::temp_dir().join(format!("harness_grep_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let file = root.join("needle.txt");
        fs::write(&file, "alpha\nHARNESS_GREP_UNIQUE\nomega\n").unwrap();

        let out = grep_in_workspace(
            &root,
            &root,
            &GrepOptions {
                pattern: "HARNESS_GREP_UNIQUE".into(),
                ignore_case: false,
                max_results: 50,
                glob: None,
            },
        )
        .unwrap();

        assert!(out.contains("needle.txt:2:HARNESS_GREP_UNIQUE"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ignore_case_works() {
        let root = std::env::temp_dir().join(format!("harness_grep_ic_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "Hello World\n").unwrap();

        let out = grep_in_workspace(
            &root,
            &root,
            &GrepOptions {
                pattern: "hello".into(),
                ignore_case: true,
                max_results: 10,
                glob: None,
            },
        )
        .unwrap();
        assert!(out.contains("Hello World"));
        let _ = fs::remove_dir_all(&root);
    }
}
