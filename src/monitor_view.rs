//! events.jsonl を読むコンテキスト監視ビューア HTML。

use std::fs;
use std::path::Path;

const MAX_EMBEDDED_TURNS: usize = 80;

const MONITOR_HTML_TEMPLATE: &str = include_str!("monitor_view/template.html");

/// ログファイルから直近の JSONL 行を読み、パースできたものだけ返す（時系列昇順）。
pub fn load_recent_event_lines(path: &Path, max: usize) -> Vec<serde_json::Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut parsed: Vec<serde_json::Value> = text
        .lines()
        .rev()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            serde_json::from_str(line).ok()
        })
        .take(max)
        .collect();
    parsed.reverse();
    parsed
}

/// 監視用 HTML（ターン一覧 + タイムライン + 折りたたみ preview）。
pub fn format_events_monitor_html(events: &[serde_json::Value], events_path: &str) -> String {
    let events_json = serde_json::to_string(events).unwrap_or_else(|_| "[]".into());
    let events_json_safe = events_json
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let path_json = serde_json::to_string(events_path).unwrap_or_else(|_| "\"\"".into());
    let path_disp = html_escape(events_path);

    MONITOR_HTML_TEMPLATE
        .replace("/*__EVENTS_JSON__*/null", &events_json_safe)
        .replace("__EVENTS_PATH_DISPLAY__", &path_disp)
        .replace("__EVENTS_PATH_JSON__", &path_json)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `events_path` があれば直近行を埋め込み、ビューア HTML を返す。
pub fn render_monitor_html_from_log(events_path: Option<&Path>) -> String {
    let (events, label) = match events_path {
        Some(p) => (
            load_recent_event_lines(p, MAX_EMBEDDED_TURNS),
            p.display().to_string(),
        ),
        None => (Vec::new(), "(no events path)".into()),
    };
    format_events_monitor_html(&events, &label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn viewer_html_has_turn_list_and_timeline_markers() {
        let events = vec![serde_json::json!({
            "v": 1,
            "ts": "2026-09-01T01:00:00.000Z",
            "kind": "turn.summary",
            "user_input": "hello",
            "steps_used": 1,
            "phases": ["react"],
            "context": {"total_tokens": 10, "llm_calls": 1},
            "steps": [{
                "step": 1,
                "phase": "react",
                "step_kind": "answer",
                "prompt_preview": "system: hi",
                "completion_preview": "{\"step\":\"answer\"}",
                "prompt_tokens": 5,
                "completion_tokens": 5
            }]
        })];
        let html = format_events_monitor_html(&events, "logs/events.jsonl");
        assert!(html.contains("data-monitor-viewer=\"events-v1\""));
        assert!(html.contains("data-turn-list"));
        assert!(html.contains("data-timeline"));
        assert!(html.contains("ターン一覧"));
        assert!(html.contains("\"user_input\":\"hello\""));
        assert!(html.contains("details.prompt"));
        assert!(!html.contains("/*__EVENTS_JSON__*/null"));
    }

    #[test]
    fn load_recent_event_lines_reads_tail() {
        let dir = std::env::temp_dir().join(format!("hs_mon_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "{}", r#"{"v":1,"user_input":"a","kind":"turn.summary"}"#).unwrap();
        writeln!(f, "{}", r#"{"v":1,"user_input":"b","kind":"turn.summary"}"#).unwrap();
        let got = load_recent_event_lines(&path, 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["user_input"], "b");
        let _ = fs::remove_dir_all(&dir);
    }
}
