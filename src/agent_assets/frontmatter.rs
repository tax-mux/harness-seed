//! `---` 区切り YAML frontmatter の分割。

use serde_yaml::Value;

#[derive(Debug, Clone)]
pub struct FrontmatterDoc {
    pub meta: Value,
    pub body: String,
}

/// Markdown 先頭の YAML frontmatter をパースする（Anthropic / Cursor スキル形式）。
pub fn parse_frontmatter(text: &str) -> Option<FrontmatterDoc> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let rest = trimmed.strip_prefix("---")?;
    let rest = rest.strip_prefix('\n').or_else(|| rest.strip_prefix("\r\n"))?;
    let end = rest.find("\n---")?;
    let yaml = &rest[..end];
    let body = rest[end + 4..].strip_prefix('\n').unwrap_or(&rest[end + 4..]);
    let meta: Value = serde_yaml::from_str(yaml).ok()?;
    Some(FrontmatterDoc {
        meta,
        body: body.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_yaml_frontmatter() {
        let text = "---\nname: demo\ndescription: hi\n---\n\n# Body\n";
        let doc = parse_frontmatter(text).expect("frontmatter");
        assert_eq!(doc.meta["name"], "demo");
        assert!(doc.body.contains("# Body"));
    }
}
