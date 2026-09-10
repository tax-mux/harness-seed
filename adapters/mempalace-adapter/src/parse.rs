use serde_json::Value;

use crate::types::{DiaryReadEntry, SearchHit};

/// search レスポンスを柔軟に解釈する。
/// `similarity` / `score` が負のヒットは距離が遠いとみなし除外する。
pub fn parse_search_hits(value: &Value) -> Vec<SearchHit> {
    let hits = parse_search_hits_raw(value);
    hits.into_iter()
        .filter(|h| !is_negative_similarity(&h.score))
        .collect()
}

fn parse_search_hits_raw(value: &Value) -> Vec<SearchHit> {
    if let Some(arr) = value
        .get("results")
        .or_else(|| value.get("hits"))
        .or_else(|| value.get("drawers"))
        .and_then(|v| v.as_array())
    {
        return arr.iter().filter_map(parse_one_hit).collect();
    }
    if let Some(arr) = value.as_array() {
        return arr.iter().filter_map(parse_one_hit).collect();
    }
    if let Some(text) = extract_text_content(value) {
        return parse_hits_from_text(&text);
    }
    if let Some(obj) = value.as_object() {
        if obj.contains_key("content") || obj.contains_key("text") || obj.contains_key("body") {
            if let Some(hit) = parse_one_hit(value) {
                return vec![hit];
            }
        }
    }
    Vec::new()
}

/// chromadb 由来の `1 - distance`。負は距離 > 1 で明らかに遠い。
pub fn is_negative_similarity(score: &Option<String>) -> bool {
    match score.as_ref().and_then(|s| s.parse::<f64>().ok()) {
        Some(v) => v < 0.0,
        None => false,
    }
}

/// diary_read レスポンスを柔軟に解釈する。
pub fn parse_diary_entries(value: &Value) -> Vec<DiaryReadEntry> {
    if let Some(arr) = value
        .get("entries")
        .or_else(|| value.get("results"))
        .or_else(|| value.get("diary"))
        .and_then(|v| v.as_array())
    {
        return arr.iter().filter_map(parse_one_diary).collect();
    }
    if let Some(arr) = value.as_array() {
        return arr.iter().filter_map(parse_one_diary).collect();
    }
    if let Some(text) = extract_text_content(value) {
        return text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, line)| DiaryReadEntry {
                title: format!("diary#{}", i + 1),
                body: line.to_string(),
                ref_id: Some(format!("diary#{}", i + 1)),
            })
            .collect();
    }
    Vec::new()
}

/// MCP tools/call の result から中身の JSON/テキストを取り出す。
pub fn unwrap_tool_result(value: &Value) -> Value {
    if let Some(err) = value.get("error") {
        return Value::Object(
            [("error".into(), err.clone())]
                .into_iter()
                .collect(),
        );
    }
    if let Some(result) = value.get("result") {
        return unwrap_tool_result(result);
    }
    if let Some(content) = value.get("content").and_then(|c| c.as_array()) {
        let mut texts = Vec::new();
        for part in content {
            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                texts.push(t.to_string());
            }
        }
        if texts.len() == 1 {
            let t = &texts[0];
            if let Ok(v) = serde_json::from_str::<Value>(t) {
                return v;
            }
            return Value::String(t.clone());
        }
        if !texts.is_empty() {
            return Value::String(texts.join("\n"));
        }
    }
    value.clone()
}

fn parse_one_hit(v: &Value) -> Option<SearchHit> {
    if let Some(s) = v.as_str() {
        return Some(SearchHit {
            title: truncate(s, 80),
            body: s.to_string(),
            ref_id: None,
            score: None,
        });
    }
    let body = v
        .get("content")
        .or_else(|| v.get("text"))
        .or_else(|| v.get("body"))
        .or_else(|| v.get("snippet"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let wing = v.get("wing").and_then(|x| x.as_str());
    let room = v.get("room").and_then(|x| x.as_str());
    let title = v
        .get("title")
        .or_else(|| v.get("name"))
        .or_else(|| v.get("path"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .or_else(|| match (wing, room) {
            (Some(w), Some(r)) => Some(format!("{w} / {r}")),
            (Some(w), None) => Some(w.to_string()),
            _ => None,
        })
        .unwrap_or_else(|| truncate(&body, 80));
    if title.is_empty() && body.is_empty() {
        return None;
    }
    // source_file を優先（`*:diary` など種別タグ。id は content hash で種別が消える）
    let ref_id = v
        .get("source_file")
        .or_else(|| v.get("id"))
        .or_else(|| v.get("ref"))
        .or_else(|| v.get("drawer_id"))
        .or_else(|| v.get("path"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let score = v
        .get("score")
        .or_else(|| v.get("similarity"))
        .map(|x| match x {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => x.to_string(),
        });
    Some(SearchHit {
        title: if title.is_empty() {
            truncate(&body, 80)
        } else {
            title
        },
        body,
        ref_id,
        score,
    })
}

fn parse_one_diary(v: &Value) -> Option<DiaryReadEntry> {
    if let Some(s) = v.as_str() {
        return Some(DiaryReadEntry {
            title: truncate(s, 80),
            body: s.to_string(),
            ref_id: None,
        });
    }
    let body = v
        .get("entry")
        .or_else(|| v.get("content"))
        .or_else(|| v.get("text"))
        .or_else(|| v.get("body"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let title = v
        .get("topic")
        .or_else(|| v.get("title"))
        .or_else(|| v.get("date"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| truncate(&body, 80));
    if title.is_empty() && body.is_empty() {
        return None;
    }
    let ref_id = v
        .get("id")
        .or_else(|| v.get("ref"))
        .or_else(|| v.get("timestamp"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    Some(DiaryReadEntry {
        title: if title.is_empty() {
            truncate(&body, 80)
        } else {
            title
        },
        body,
        ref_id,
    })
}

fn extract_text_content(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(content) = value.get("content").and_then(|c| c.as_array()) {
        let mut texts = Vec::new();
        for part in content {
            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                texts.push(t.to_string());
            }
        }
        if !texts.is_empty() {
            return Some(texts.join("\n"));
        }
    }
    value
        .get("text")
        .or_else(|| value.get("message"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn parse_hits_from_text(text: &str) -> Vec<SearchHit> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, line)| SearchHit {
            title: truncate(line, 80),
            body: line.to_string(),
            ref_id: Some(format!("hit#{}", i + 1)),
            score: None,
        })
        .collect()
}

fn truncate(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let snippet: String = s.chars().take(max_chars).collect();
    format!("{snippet}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_results_array() {
        let v = json!({
            "results": [
                {"id": "d1", "title": "T", "content": "body", "score": 0.9}
            ]
        });
        let hits = parse_search_hits(&v);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].ref_id.as_deref(), Some("d1"));
        assert!(hits[0].body.contains("body"));
    }

    #[test]
    fn parses_mempalace_search_memories_shape() {
        let v = json!({
            "results": [{
                "text": "drawer body",
                "wing": "OpenHarness",
                "room": "architecture",
                "source_file": "x.md",
                "similarity": 0.8
            }]
        });
        let hits = parse_search_hits(&v);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "OpenHarness / architecture");
        assert_eq!(hits[0].body, "drawer body");
    }

    #[test]
    fn drops_negative_similarity_hits() {
        let v = json!({
            "results": [
                {"text": "near", "wing": "w", "room": "a", "similarity": 0.1},
                {"text": "far", "wing": "w", "room": "b", "similarity": -0.4},
                {"text": "zero", "wing": "w", "room": "c", "similarity": 0.0}
            ]
        });
        let hits = parse_search_hits(&v);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.body != "far"));
    }

    #[test]
    fn unwraps_mcp_content_json() {
        let inner = json!({"results": [{"id": "x", "text": "hello"}]});
        let v = json!({
            "result": {
                "content": [{"type": "text", "text": inner.to_string()}]
            }
        });
        let unwrapped = unwrap_tool_result(&v);
        let hits = parse_search_hits(&unwrapped);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].ref_id.as_deref(), Some("x"));
    }

    #[test]
    fn parses_diary_entries() {
        let v = json!({
            "entries": [
                {"topic": "general", "entry": "SESSION:built.thing"}
            ]
        });
        let entries = parse_diary_entries(&v);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "general");
    }
}
