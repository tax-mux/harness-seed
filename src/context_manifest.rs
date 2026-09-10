//! ワークスペース JSON マニフェストから、スコープ付き画像・テキストをコンテキストへ注入する（汎用）。

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Deserialize;
use serde_json::Value;

use crate::context::PromptBlocks;
use crate::tasks::ContextManifestSpec;

pub const SCOPED_RECALL_PREFIX: &str = "[context-manifest ";

/// LLM ビジョン API 向け画像添付。
#[derive(Debug, Clone)]
pub struct VisionAttachment {
    pub entry_id: String,
    pub path: PathBuf,
    pub mime: String,
    pub base64: String,
}

#[derive(Debug, Deserialize)]
struct ContextManifestFile {
    #[serde(default)]
    entries: Vec<ContextManifestEntry>,
}

#[derive(Debug, Deserialize)]
struct ContextManifestEntry {
    id: String,
    #[serde(default)]
    scope: Value,
    #[serde(default)]
    images: Vec<ManifestImage>,
    #[serde(default)]
    recalled: Vec<ManifestRecalled>,
}

#[derive(Debug, Deserialize)]
struct ManifestImage {
    path: String,
}

#[derive(Debug, Deserialize)]
struct ManifestRecalled {
    path: String,
    #[serde(default)]
    label: String,
}

#[derive(Debug)]
pub enum ContextManifestError {
    Read { path: PathBuf, source: std::io::Error },
    Parse { path: PathBuf, source: serde_json::Error },
    ImageRead { path: PathBuf, source: std::io::Error },
    EntryNotFound { scope: Value },
    NotConfigured,
}

impl fmt::Display for ContextManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "read {}: {source}", path.display()),
            Self::Parse { path, source } => write!(f, "parse {}: {source}", path.display()),
            Self::ImageRead { path, source } => write!(f, "image {}: {source}", path.display()),
            Self::EntryNotFound { scope } => write!(f, "no manifest entry for scope {scope}"),
            Self::NotConfigured => write!(f, "context manifest path not configured"),
        }
    }
}

impl std::error::Error for ContextManifestError {}

fn mime_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/png",
    }
}

fn load_manifest(path: &Path) -> Result<ContextManifestFile, ContextManifestError> {
    let text = fs::read_to_string(path).map_err(|source| ContextManifestError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| ContextManifestError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// `params` のスコープキーが `entry.scope` と一致するか。
fn scope_matches(entry_scope: &Value, params: &Value, scope_params: &[String]) -> bool {
    let Some(scope_obj) = entry_scope.as_object() else {
        return scope_params.is_empty();
    };
    if scope_obj.is_empty() {
        return false;
    }
    let keys: Vec<&String> = if scope_params.is_empty() {
        scope_obj.keys().collect()
    } else {
        scope_params.iter().collect()
    };
    for key in keys {
        let expected = scope_obj.get(key).and_then(|v| v.as_str()).unwrap_or("");
        let actual = params.get(key).and_then(|v| v.as_str()).unwrap_or("");
        if expected != actual {
            return false;
        }
    }
    true
}

fn clear_scoped_context(blocks: &mut PromptBlocks) {
    blocks
        .recalled
        .retain(|chunk| !chunk.starts_with(SCOPED_RECALL_PREFIX));
    blocks.clear_vision_attachments();
}

fn read_recalled_file(path: &Path, label: &str) -> Result<String, ContextManifestError> {
    let mut text = fs::read_to_string(path).map_err(|source| ContextManifestError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    const MAX: usize = 12_000;
    if text.len() > MAX {
        text.truncate(MAX);
        text.push_str("\n...(truncated)\n");
    }
    let name = if label.trim().is_empty() {
        path.display().to_string()
    } else {
        label.to_string()
    };
    Ok(format!("### {name}\nPath: {}\n\n{text}", path.display()))
}

/// 起動時: マニフェストがあれば entry id 一覧だけ recalled に載せる（内容はサブタスク実行時）。
pub fn note_manifest_available(
    manifest_path: &Path,
    blocks: &mut PromptBlocks,
) -> Result<(), ContextManifestError> {
    if !manifest_path.is_file() {
        return Ok(());
    }
    let manifest = load_manifest(manifest_path)?;
    if manifest.entries.is_empty() {
        return Ok(());
    }
    let ids: Vec<_> = manifest.entries.iter().map(|e| e.id.as_str()).collect();
    blocks.push_recalled(format!(
        "Context manifest ({}) lists {} entr{}: {}. \
         Tasks with `context_manifest` inject scoped images (first LLM step only) and recalled files on subtask start.",
        manifest_path.display(),
        ids.len(),
        if ids.len() == 1 { "y" } else { "ies" },
        ids.join(", ")
    ));
    Ok(())
}

/// タスク定義の `context_manifest` に従い、params でスコープした entry を注入する。
pub fn apply_scoped_entry(
    manifest_path: &Path,
    spec: &ContextManifestSpec,
    params: &Value,
    blocks: &mut PromptBlocks,
) -> Result<usize, ContextManifestError> {
    if !manifest_path.is_file() {
        return Ok(0);
    }

    let manifest = load_manifest(manifest_path)?;
    let entry = manifest
        .entries
        .iter()
        .find(|e| scope_matches(&e.scope, params, &spec.scope_params))
        .ok_or_else(|| ContextManifestError::EntryNotFound {
            scope: params.clone(),
        })?;

    clear_scoped_context(blocks);

    let mut attachments = Vec::new();
    for image in &entry.images {
        let path = PathBuf::from(&image.path);
        let bytes = fs::read(&path).map_err(|source| ContextManifestError::ImageRead {
            path: path.clone(),
            source,
        })?;
        attachments.push(VisionAttachment {
            entry_id: entry.id.clone(),
            path: path.clone(),
            mime: mime_for_path(&path).to_string(),
            base64: STANDARD.encode(&bytes),
        });
    }
    blocks.set_vision_attachments(attachments.clone());

    let mut recalled_body = String::new();
    for item in &entry.recalled {
        let path = PathBuf::from(&item.path);
        recalled_body.push_str(&read_recalled_file(&path, &item.label)?);
        recalled_body.push_str("\n\n");
    }

    if !recalled_body.trim().is_empty() || !attachments.is_empty() {
        blocks.push_recalled(format!(
            "{SCOPED_RECALL_PREFIX}{}]\n{recalled_body}",
            entry.id
        ));
    }

    Ok(attachments.len())
}

/// マニフェスト注入失敗時に mission へ追記するヒント（汎用）。
pub fn format_apply_error_hint(err: &ContextManifestError, spec: &ContextManifestSpec) -> String {
    format!(
        "\n\n## Context manifest\nFailed to inject scoped context: {err}\n\
         Ensure subtask `params` include scope keys {:?} matching a manifest entry, \
         and run the upstream step that generates the manifest file.\n",
        spec.scope_params
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_fixture(root: &Path) {
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(root.join("data/note.txt"), "hello context").unwrap();
        let png_path = root.join("data/page.png");
        let png_bytes: [u8; 67] = [
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        fs::write(&png_path, png_bytes).unwrap();

        let manifest = format!(
            r#"{{
              "entries": [{{
                "id": "demo/a",
                "scope": {{ "region": "demo", "stem": "a" }},
                "images": [{{ "path": "{}" }}],
                "recalled": [{{ "path": "{}", "label": "note" }}]
              }}]
            }}"#,
            png_path.display().to_string().replace('\\', "\\\\"),
            root.join("data/note.txt")
                .display()
                .to_string()
                .replace('\\', "\\\\")
        );
        fs::write(root.join("manifest.json"), manifest).unwrap();
    }

    #[test]
    fn apply_scoped_entry_injects_image_and_recalled() {
        let root = std::env::temp_dir().join(format!("hs-ctx-manifest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_fixture(&root);

        let spec = ContextManifestSpec {
            scope_params: vec!["region".into(), "stem".into()],
        };
        let params = serde_json::json!({ "region": "demo", "stem": "a" });
        let mut blocks = PromptBlocks::new();
        let n = apply_scoped_entry(
            &root.join("manifest.json"),
            &spec,
            &params,
            &mut blocks,
        )
        .unwrap();
        assert_eq!(n, 1);
        assert_eq!(blocks.vision_attachments.len(), 1);
        assert!(blocks.recalled.iter().any(|c| c.contains("hello context")));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_does_not_clear_on_scope_miss() {
        let root = std::env::temp_dir().join(format!("hs-ctx-miss-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_fixture(&root);

        let spec = ContextManifestSpec {
            scope_params: vec!["region".into(), "stem".into()],
        };
        let mut blocks = PromptBlocks::new();
        blocks.push_recalled("keep me");
        let err = apply_scoped_entry(
            &root.join("manifest.json"),
            &spec,
            &serde_json::json!({}),
            &mut blocks,
        )
        .unwrap_err();
        assert!(matches!(err, ContextManifestError::EntryNotFound { .. }));
        assert!(blocks.recalled.iter().any(|c| c.contains("keep me")));

        let _ = fs::remove_dir_all(&root);
    }
}
