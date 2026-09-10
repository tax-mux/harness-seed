use serde_json::{json, Value};

use crate::config::{MempalaceConfig, MempalaceProtocol};
use crate::mcp_stdio::McpStdioTransport;
use crate::parse::{parse_search_hits, unwrap_tool_result};
use crate::types::{DiaryReadEntry, MempalaceError, SearchHit};
use crate::{TOOL_ADD_DRAWER, TOOL_LIST_WINGS, TOOL_SEARCH};

/// ツール呼び出しの下位トランスポート。
pub trait MempalaceTransport: Send {
    fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, MempalaceError>;
}

/// reqwest による HTTP トランスポート。
pub struct HttpTransport {
    client: reqwest::blocking::Client,
    config: MempalaceConfig,
}

impl HttpTransport {
    pub fn new(config: MempalaceConfig) -> Result<Self, MempalaceError> {
        config.validate()?;
        let client = reqwest::blocking::Client::builder()
            .timeout(config.timeout())
            .build()
            .map_err(|e| MempalaceError::Http(e.to_string()))?;
        Ok(Self { client, config })
    }
}

impl MempalaceTransport for HttpTransport {
    fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, MempalaceError> {
        let base = self.config.base_url.trim_end_matches('/');
        let (url, body) = match self.config.protocol {
            MempalaceProtocol::ToolsPath => (format!("{base}/tools/{name}"), arguments),
            MempalaceProtocol::McpJsonrpc => (
                base.to_string(),
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {
                        "name": name,
                        "arguments": arguments,
                    }
                }),
            ),
            MempalaceProtocol::McpStdio => {
                return Err(MempalaceError::Config(
                    "HttpTransport does not support mcp_stdio".into(),
                ));
            }
        };

        let mut req = self.client.post(&url).json(&body);
        if let Some(key) = &self.config.api_key {
            req = req.bearer_auth(key);
        }

        let response = req
            .send()
            .map_err(|e| MempalaceError::Http(format!("{url}: {e}")))?;
        let status = response.status();
        let text = response
            .text()
            .map_err(|e| MempalaceError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(MempalaceError::Http(format!(
                "{url} -> {status}: {}",
                truncate(&text, 200)
            )));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        if let Some(err) = value.get("error") {
            return Err(MempalaceError::Backend(err.to_string()));
        }
        Ok(unwrap_tool_result(&value))
    }
}

/// mempalace 高水準クライアント。
pub struct MempalaceClient {
    transport: Box<dyn MempalaceTransport>,
    config: MempalaceConfig,
}

impl MempalaceClient {
    /// `protocol` に応じて MCP stdio または HTTP で接続する。
    pub fn connect(config: MempalaceConfig) -> Result<Self, MempalaceError> {
        let config = MempalaceConfig::from_env_or(config);
        config.validate()?;
        let transport: Box<dyn MempalaceTransport> = match config.protocol {
            MempalaceProtocol::McpStdio => Box::new(McpStdioTransport::spawn(&config)?),
            MempalaceProtocol::ToolsPath | MempalaceProtocol::McpJsonrpc => {
                Box::new(HttpTransport::new(config.clone())?)
            }
        };
        let client = Self { transport, config };
        if let Err(err) = client.ensure_wing_initialized() {
            // palace 未作成などは警告のみ（レイヤ自体は残し、後続 diary で再試行しうる）
            eprintln!("[memory] mempalace wing init: {err}");
        }
        Ok(client)
    }

    pub fn with_transport(
        config: MempalaceConfig,
        transport: impl MempalaceTransport + 'static,
    ) -> Self {
        Self {
            transport: Box::new(transport),
            config,
        }
    }

    pub fn config(&self) -> &MempalaceConfig {
        &self.config
    }

    /// プロジェクト wing 全体を検索（全エージェントの room を共有）。
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, MempalaceError> {
        let mut args = json!({
            "query": query,
            "limit": limit.max(1) as i64,
        });
        if let Some(wing) = &self.config.wing {
            args["wing"] = json!(wing);
        }
        // room は付けない — wing 内の他エージェント room も見える
        let value = self.transport.call_tool(TOOL_SEARCH, args)?;
        Ok(parse_search_hits(&value))
    }

    /// 自エージェント room の直近 diary（`SESSION` で検索）。
    pub fn diary_read(&self, last_n: usize) -> Result<Vec<DiaryReadEntry>, MempalaceError> {
        let Some(wing) = self.config.wing.as_ref() else {
            return Ok(vec![]);
        };
        let room = self.config.agent_room();
        let args = json!({
            "query": "SESSION",
            "limit": last_n.max(1) as i64,
            "wing": wing,
            "room": room,
        });
        let value = self.transport.call_tool(TOOL_SEARCH, args)?;
        Ok(parse_diary_entries_from_search(&value))
    }

    /// 自エージェント room に diary を書く（`wing_{project}` / `room={agent}`）。
    pub fn diary_write(&self, entry: &str, topic: Option<&str>) -> Result<(), MempalaceError> {
        let Some(wing) = self.config.wing.as_ref() else {
            return Err(MempalaceError::Config(
                "wing is required for diary_write".into(),
            ));
        };
        let room = self.config.agent_room();
        let mut content = entry.to_string();
        if let Some(topic) = topic {
            if !content.contains("topic:") {
                content = format!("topic:{topic}|{content}");
            }
        }
        let result = self.transport.call_tool(
            TOOL_ADD_DRAWER,
            json!({
                "wing": wing,
                "room": room,
                "content": content,
                "source_file": "harness-seed:diary",
                "added_by": self.config.agent_name,
            }),
        )?;
        if let Some(err) = result.get("error") {
            return Err(MempalaceError::Backend(err.to_string()));
        }
        if result.get("success") == Some(&Value::Bool(false)) {
            return Err(MempalaceError::Backend(result.to_string()));
        }
        Ok(())
    }

    /// 対象 wing が無いとき、`_meta` room に最小シードを書いて初期化する。
    /// （エージェント room には書かない — 検索でプロダクト説明と誤認されないようにする）
    pub fn ensure_wing_initialized(&self) -> Result<(), MempalaceError> {
        if !self.config.init_wing_if_missing {
            return Ok(());
        }
        let Some(wing) = self.config.wing.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty())
        else {
            return Ok(());
        };

        let listed = self.transport.call_tool(TOOL_LIST_WINGS, json!({}))?;
        if let Some(err) = listed.get("error") {
            return Err(MempalaceError::Backend(format!(
                "list_wings failed (is palace initialized?): {err}"
            )));
        }
        if wing_exists(&listed, wing) {
            return Ok(());
        }

        // 意味の薄いマーカーのみ（LLM が「harness-seed の説明」と取り違えない）
        let content = format!("META:wing-init wing={wing}");
        let result = self.transport.call_tool(
            TOOL_ADD_DRAWER,
            json!({
                "wing": wing,
                "room": "_meta",
                "content": content,
                "source_file": "harness-seed:wing-init",
                "added_by": "harness-seed",
            }),
        )?;
        if let Some(err) = result.get("error") {
            return Err(MempalaceError::Backend(format!(
                "init wing {wing}: {err}"
            )));
        }
        if result.get("success") == Some(&Value::Bool(false)) {
            return Err(MempalaceError::Backend(format!(
                "init wing {wing}: {result}"
            )));
        }
        eprintln!("[memory] mempalace created wing={wing} room=_meta");
        Ok(())
    }
}

fn parse_diary_entries_from_search(value: &Value) -> Vec<DiaryReadEntry> {
    parse_search_hits(value)
        .into_iter()
        .map(|h| DiaryReadEntry {
            title: h.title,
            body: h.body,
            ref_id: h.ref_id,
        })
        .collect()
}

fn wing_exists(list_wings_result: &Value, wing: &str) -> bool {
    list_wings_result
        .get("wings")
        .and_then(|w| w.as_object())
        .is_some_and(|m| m.contains_key(wing))
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
    use std::sync::{Arc, Mutex};

    struct FakeTransport {
        calls: Arc<Mutex<Vec<(String, Value)>>>,
        response: Value,
    }

    impl MempalaceTransport for FakeTransport {
        fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, MempalaceError> {
            self.calls
                .lock()
                .unwrap()
                .push((name.to_string(), arguments));
            Ok(self.response.clone())
        }
    }

    #[test]
    fn search_maps_results() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transport = FakeTransport {
            calls: calls.clone(),
            response: json!({
                "results": [
                    {"id": "1", "title": "hit", "content": "body text"}
                ]
            }),
        };
        let client = MempalaceClient::with_transport(MempalaceConfig::default(), transport);
        let hits = client.search("q", 3).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "hit");
        let logged = calls.lock().unwrap();
        assert_eq!(logged[0].0, TOOL_SEARCH);
        assert_eq!(logged[0].1["query"], "q");
    }

    #[test]
    fn diary_write_uses_project_wing_and_agent_room() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transport = FakeTransport {
            calls: calls.clone(),
            response: json!({"success": true}),
        };
        let mut cfg = MempalaceConfig::default();
        cfg.agent_name = "agent-a".into();
        cfg.wing = Some("wing_harness-seed".into());
        let client = MempalaceClient::with_transport(cfg, transport);
        client.diary_write("SESSION:test", Some("harness")).unwrap();
        let logged = calls.lock().unwrap();
        assert_eq!(logged[0].0, TOOL_ADD_DRAWER);
        assert_eq!(logged[0].1["wing"], "wing_harness-seed");
        assert_eq!(logged[0].1["room"], "agent-a");
        assert!(logged[0].1["content"]
            .as_str()
            .unwrap()
            .contains("SESSION:test"));
    }

    #[test]
    fn diary_read_searches_agent_room() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transport = FakeTransport {
            calls: calls.clone(),
            response: json!({
                "results": [{"text": "SESSION:line", "wing": "wing_p", "room": "agent-a"}]
            }),
        };
        let mut cfg = MempalaceConfig::default();
        cfg.wing = Some("wing_p".into());
        cfg.agent_name = "agent-a".into();
        let client = MempalaceClient::with_transport(cfg, transport);
        let entries = client.diary_read(2).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].body.contains("SESSION:line"));
        let logged = calls.lock().unwrap();
        assert_eq!(logged[0].0, TOOL_SEARCH);
        assert_eq!(logged[0].1["room"], "agent-a");
        assert_eq!(logged[0].1["wing"], "wing_p");
    }

    #[test]
    fn search_does_not_filter_by_agent_room() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transport = FakeTransport {
            calls: calls.clone(),
            response: json!({"results": []}),
        };
        let mut cfg = MempalaceConfig::default();
        cfg.wing = Some("wing_p".into());
        cfg.agent_name = "agent-a".into();
        let client = MempalaceClient::with_transport(cfg, transport);
        let _ = client.search("q", 3).unwrap();
        let logged = calls.lock().unwrap();
        assert!(logged[0].1.get("room").is_none());
        assert_eq!(logged[0].1["wing"], "wing_p");
    }
}
