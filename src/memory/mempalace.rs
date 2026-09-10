//! mempalace-adapter を [`MemoryBridge`] に接続する。

use mempalace_adapter::{MempalaceClient, MempalaceConfig, MempalaceError};

use super::{DiaryEntry, MemoryBridge, MemoryError, RecalledItem, RecalledSource};

/// mempalace（MCP stdio / HTTP）バックエンドの [`MemoryBridge`]。
pub struct MempalaceBridge {
    client: MempalaceClient,
}

impl MempalaceBridge {
    pub fn connect(config: MempalaceConfig) -> Result<Self, MemoryError> {
        let client = MempalaceClient::connect(config).map_err(map_err)?;
        Ok(Self { client })
    }

    pub fn with_client(client: MempalaceClient) -> Self {
        Self { client }
    }

    pub fn scope_label(&self) -> String {
        self.client.config().scope_label()
    }
}

impl MemoryBridge for MempalaceBridge {
    fn recent_work(&self, max_entries: usize) -> Result<Vec<RecalledItem>, MemoryError> {
        // 自 agent room の直近 diary（SESSION）。search とは別に必ず載せる。
        let entries = self.client.diary_read(max_entries).map_err(map_err)?;
        Ok(entries
            .into_iter()
            .map(|e| RecalledItem {
                title: e.title,
                body: e.body,
                source: RecalledSource::RecentWork,
                ref_id: e.ref_id,
            })
            .collect())
    }

    fn search(&self, query: &str, top_k: usize) -> Result<Vec<RecalledItem>, MemoryError> {
        // wing 全体を検索し、diary 由来と knowledge 由来を交互に混ぜる
        // （片方が上位を独占してもう片方を押しのけないようにする）
        let fetch = top_k.saturating_mul(3).max(top_k);
        let hits = self.client.search(query, fetch).map_err(map_err)?;
        let (diary, knowledge): (Vec<_>, Vec<_>) =
            hits.into_iter().partition(is_diary_hit);
        Ok(interleave_take(knowledge, diary, top_k)
            .into_iter()
            .map(|h| {
                let mut body = h.body;
                if let Some(score) = h.score {
                    body = format!("(score: {score})\n{body}");
                }
                RecalledItem {
                    title: h.title,
                    body,
                    source: RecalledSource::SearchHit,
                    ref_id: h.ref_id,
                }
            })
            .collect())
    }

    fn diary(&mut self, entry: &DiaryEntry) -> Result<(), MemoryError> {
        let text = format_diary_aaak(entry);
        self.client
            .diary_write(&text, Some("harness-seed"))
            .map_err(map_err)
    }
}

fn format_diary_aaak(entry: &DiaryEntry) -> String {
    let mut parts = Vec::new();
    let date = chrono_like_today();
    // mempalace add_drawer は content[:100] で ID を決めるため、先頭をユニークにする
    parts.push(format!("id:{}", unique_stamp()));
    parts.push(format!("SESSION:{date}"));
    if !entry.summary.trim().is_empty() {
        parts.push(format!("summary:{}", compress_token(&entry.summary)));
    }
    if !entry.user_input.trim().is_empty() {
        parts.push(format!("user:{}", compress_token(&entry.user_input)));
    }
    if !entry.phases.is_empty() {
        let phase_bits: Vec<String> = entry
            .phases
            .iter()
            .map(|p| format!("p{}:{}", p.id, compress_token(&p.goal)))
            .collect();
        parts.push(format!("phases:{}", phase_bits.join("+")));
    } else if !entry.answer.trim().is_empty() {
        parts.push(format!("answer:{}", compress_token(&entry.answer)));
    }
    parts.join("|")
}

fn unique_stamp() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn compress_token(s: &str) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(".");
    let count = t.chars().count();
    if count <= 120 {
        t
    } else {
        let snippet: String = t.chars().take(120).collect();
        format!("{snippet}…")
    }
}

fn chrono_like_today() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

fn map_err(err: MempalaceError) -> MemoryError {
    MemoryError::Backend(err.to_string())
}

/// diary 書き込み由来かどうか（`source_file=*:diary`、または SESSION 本文）。
fn is_diary_hit(h: &mempalace_adapter::SearchHit) -> bool {
    if h.ref_id
        .as_deref()
        .is_some_and(|r| r.ends_with(":diary") || r == "diary")
    {
        return true;
    }
    // source_file が id に潰れていても、自前 diary フォーマットなら SESSION: が付く
    h.body.contains("SESSION:")
}

/// knowledge を先に1件、diary を1件…と交互に取り、片方だけが尽きたら残りで埋める。
fn interleave_take<T>(mut knowledge: Vec<T>, mut diary: Vec<T>, limit: usize) -> Vec<T> {
    knowledge.reverse();
    diary.reverse();
    let mut out = Vec::with_capacity(limit);
    while out.len() < limit {
        let mut added = false;
        if let Some(h) = knowledge.pop() {
            out.push(h);
            added = true;
        }
        if out.len() >= limit {
            break;
        }
        if let Some(h) = diary.pop() {
            out.push(h);
            added = true;
        }
        if !added {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mempalace_adapter::{MempalaceConfig, MempalaceTransport, SearchHit};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    #[test]
    fn interleave_mixes_knowledge_and_diary() {
        let knowledge = vec!["k1", "k2", "k3"];
        let diary = vec!["d1", "d2"];
        assert_eq!(
            interleave_take(knowledge, diary, 4),
            vec!["k1", "d1", "k2", "d2"]
        );
    }

    #[test]
    fn interleave_fills_from_remaining_side() {
        let knowledge = vec!["k1"];
        let diary = vec!["d1", "d2", "d3"];
        assert_eq!(
            interleave_take(knowledge, diary, 4),
            vec!["k1", "d1", "d2", "d3"]
        );
    }

    #[test]
    fn diary_hit_detects_source_file() {
        let h = SearchHit {
            title: "w / r".into(),
            body: "x".into(),
            ref_id: Some("harness-seed:diary".into()),
            score: Some("0.1".into()),
        };
        assert!(is_diary_hit(&h));
        let k = SearchHit {
            title: "w / cursor-agent".into(),
            body: "overview".into(),
            ref_id: Some("cursor-agent:harness-seed-overview".into()),
            score: Some("0.2".into()),
        };
        assert!(!is_diary_hit(&k));
    }

    struct FakeTransport {
        response: Value,
        writes: Arc<Mutex<Vec<Value>>>,
    }

    impl MempalaceTransport for FakeTransport {
        fn call_tool(
            &self,
            name: &str,
            arguments: Value,
        ) -> Result<Value, MempalaceError> {
            if name == mempalace_adapter::TOOL_ADD_DRAWER {
                self.writes.lock().unwrap().push(arguments);
                return Ok(json!({"success": true}));
            }
            if name == mempalace_adapter::TOOL_SEARCH {
                return Ok(self.response.clone());
            }
            Ok(Value::Null)
        }
    }

    #[test]
    fn bridge_search_and_recent_work() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let transport = FakeTransport {
            response: json!({
                "results": [{"id": "s1", "title": "hit", "content": "body"}]
            }),
            writes: writes.clone(),
        };
        let mut cfg = MempalaceConfig::default();
        cfg.wing = Some("wing_harness-seed".into());
        cfg.agent_name = "harness-seed".into();
        let client = MempalaceClient::with_transport(cfg, transport);
        let mut bridge = MempalaceBridge::with_client(client);
        let recent = bridge.recent_work(3).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].source, RecalledSource::RecentWork);
        let hits = bridge.search("q", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].ref_id.as_deref(), Some("s1"));
        bridge
            .diary(&DiaryEntry {
                user_input: "do thing".into(),
                summary: "done".into(),
                answer: "ok".into(),
                phases: vec![],
            })
            .unwrap();
        let w = writes.lock().unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0]["wing"], "wing_harness-seed");
        assert_eq!(w[0]["room"], "harness-seed");
        assert!(w[0]["content"].as_str().unwrap().contains("SESSION:"));
    }

    #[test]
    fn bridge_search_interleaves_knowledge_and_diary() {
        let transport = FakeTransport {
            response: json!({
                "results": [
                    {"text": "d1", "wing": "w", "room": "harness-seed",
                     "source_file": "harness-seed:diary", "similarity": 0.9},
                    {"text": "d2", "wing": "w", "room": "harness-seed",
                     "source_file": "harness-seed:diary", "similarity": 0.8},
                    {"text": "k1", "wing": "w", "room": "cursor-agent",
                     "source_file": "cursor-agent:overview", "similarity": 0.7},
                    {"text": "k2", "wing": "w", "room": "cursor-agent",
                     "source_file": "cursor-agent:notes", "similarity": 0.6}
                ]
            }),
            writes: Arc::new(Mutex::new(Vec::new())),
        };
        let mut cfg = MempalaceConfig::default();
        cfg.wing = Some("wing_harness-seed".into());
        cfg.agent_name = "harness-seed".into();
        let client = MempalaceClient::with_transport(cfg, transport);
        let bridge = MempalaceBridge::with_client(client);
        let hits = bridge.search("q", 4).unwrap();
        assert_eq!(hits.len(), 4);
        // knowledge 先・diary 後を交互（スコア順は各バケット内で維持）
        assert_eq!(hits[0].body.lines().last(), Some("k1"));
        assert_eq!(hits[1].body.lines().last(), Some("d1"));
        assert_eq!(hits[2].body.lines().last(), Some("k2"));
        assert_eq!(hits[3].body.lines().last(), Some("d2"));
    }
}
