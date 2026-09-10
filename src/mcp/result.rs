//! MCP `tools/call` 応答の正規化。

use serde_json::Value;

pub fn unwrap_tool_result(value: &Value) -> Value {
    if let Some(err) = value.get("error") {
        return Value::Object([("error".into(), err.clone())].into_iter().collect());
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

pub fn format_tool_output(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unwraps_text_content() {
        let v = json!({
            "content": [{"type": "text", "text": "hello"}]
        });
        assert_eq!(unwrap_tool_result(&v), Value::String("hello".into()));
    }
}
