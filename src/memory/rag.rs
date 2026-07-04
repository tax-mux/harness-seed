//! 記憶系 RAG（アダプタ手前）。
//!
//! 作業ログ（`recent_work`）と知識検索（`search`）を分岐してから
//! [`MemoryBridge`] を叩く。エージェントの plan/exec ループには載せない。

use super::{
    format_recalled_block, MemoryBridge, MemoryRuntimeConfig, RecalledItem, RecalledSource,
};
use crate::context::PromptBlocks;
use crate::llm::{ChatMessage, LlmConnector};
use crate::text_match::looks_like_continuation;

/// 記憶 RAG の分岐結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRoute {
    /// 作業ログ（直近 diary）を取るか。
    pub work_log: bool,
    /// 知識検索するか。
    pub knowledge: bool,
    /// 知識側の検索語（1〜数件）。`knowledge == false` なら空。
    pub queries: Vec<String>,
}

impl MemoryRoute {
    pub fn none() -> Self {
        Self {
            work_log: false,
            knowledge: false,
            queries: vec![],
        }
    }

    pub fn work_log_only() -> Self {
        Self {
            work_log: true,
            knowledge: false,
            queries: vec![],
        }
    }

    pub fn knowledge_only(queries: Vec<String>) -> Self {
        Self {
            work_log: false,
            knowledge: true,
            queries,
        }
    }
}

/// retrieve 結果（整形前）。
#[derive(Debug, Clone, Default)]
pub struct PackedRecall {
    pub route: MemoryRoute,
    pub work_log: Vec<RecalledItem>,
    pub knowledge: Vec<RecalledItem>,
}

impl Default for MemoryRoute {
    fn default() -> Self {
        Self::none()
    }
}

/// 分岐を決めるルータ（ルール / LLM）。
pub trait MemoryRouter: Send {
    fn route(&self, user_input: &str, prior_one_liner: Option<&str>) -> MemoryRoute;
}

/// continuation ヒント → 作業ログ、それ以外の非空入力 → 知識検索。
#[derive(Debug, Default, Clone, Copy)]
pub struct RuleRouter;

impl MemoryRouter for RuleRouter {
    fn route(&self, user_input: &str, _prior_one_liner: Option<&str>) -> MemoryRoute {
        let input = user_input.trim();
        if input.is_empty() {
            return MemoryRoute::none();
        }
        if looks_like_continuation(input) {
            return MemoryRoute::work_log_only();
        }
        MemoryRoute::knowledge_only(vec![input.to_string()])
    }
}

/// 小さい JSON 一発で分岐する LLM ルータ。失敗時は `fallback`。
pub struct LlmRouter<C: LlmConnector + Send, F: MemoryRouter> {
    connector: C,
    fallback: F,
    max_queries: usize,
}

impl<C: LlmConnector + Send, F: MemoryRouter> LlmRouter<C, F> {
    pub fn new(connector: C, fallback: F, max_queries: usize) -> Self {
        Self {
            connector,
            fallback,
            max_queries: max_queries.max(1),
        }
    }
}

impl<C: LlmConnector + Send, F: MemoryRouter> MemoryRouter for LlmRouter<C, F> {
    fn route(&self, user_input: &str, prior_one_liner: Option<&str>) -> MemoryRoute {
        let input = user_input.trim();
        if input.is_empty() {
            return MemoryRoute::none();
        }
        let messages = build_router_messages(input, prior_one_liner, self.max_queries);
        match self.connector.complete(&messages) {
            Ok(result) => match parse_route_json(&result.content, self.max_queries) {
                Some(route) => route,
                None => {
                    eprintln!("[memory.rag] router parse failed; using fallback");
                    self.fallback.route(user_input, prior_one_liner)
                }
            },
            Err(err) => {
                eprintln!("[memory.rag] router llm error: {err}; using fallback");
                self.fallback.route(user_input, prior_one_liner)
            }
        }
    }
}

const ROUTER_SYSTEM: &str = r#"You are a memory-RAG router (not the main agent). Decide whether to load work logs and/or run knowledge search.
Reply with ONE JSON object only (no markdown):
{"work_log":<bool>,"knowledge":<bool>,"queries":["<search term>",...]}

Rules:
- work_log=true only for continuing prior work ("続き", "もっと", "same task", follow-ups that need the last session diary).
- knowledge=true for questions/facts/explanations that need memory search. Put 1..N short search terms in queries (keywords, not full sentences).
- Topic change: work_log=false. Do not attach previous work logs.
- Greetings / no memory needed: both false, queries=[].
- Do not set both true unless the user clearly continues work AND needs extra knowledge.
- queries must be empty when knowledge=false.
"#;

fn build_router_messages(
    user_input: &str,
    prior_one_liner: Option<&str>,
    max_queries: usize,
) -> Vec<ChatMessage> {
    let mut user = String::new();
    user.push_str("Memory RAG route request.\n");
    user.push_str(&format!("max_queries: {max_queries}\n"));
    if let Some(prior) = prior_one_liner.map(str::trim).filter(|s| !s.is_empty()) {
        user.push_str("prior_turn_one_liner: ");
        user.push_str(prior);
        user.push('\n');
    }
    user.push_str("user_input: ");
    user.push_str(user_input);
    user.push('\n');
    user.push_str("JSON:");
    vec![
        ChatMessage::system(ROUTER_SYSTEM),
        ChatMessage::user(user),
    ]
}

fn parse_route_json(raw: &str, max_queries: usize) -> Option<MemoryRoute> {
    let trimmed = raw.trim();
    let json_str = extract_json_object(trimmed)?;
    let value: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let work_log = value.get("work_log").and_then(|v| v.as_bool()).unwrap_or(false);
    let knowledge = value
        .get("knowledge")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut queries = Vec::new();
    if knowledge {
        if let Some(arr) = value.get("queries").and_then(|v| v.as_array()) {
            for q in arr {
                if let Some(s) = q.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    queries.push(s.to_string());
                    if queries.len() >= max_queries {
                        break;
                    }
                }
            }
        }
        if queries.is_empty() {
            // knowledge なのに queries 欠落 → フォールバック不能なので None
            return None;
        }
    }
    Some(MemoryRoute {
        work_log,
        knowledge,
        queries,
    })
}

fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end < start {
        return None;
    }
    Some(&s[start..=end])
}

/// アダプタ手前の記憶 RAG。
pub struct MemoryRag {
    router: Box<dyn MemoryRouter>,
    max_queries: usize,
}

impl MemoryRag {
    pub fn with_router(router: Box<dyn MemoryRouter>, max_queries: usize) -> Self {
        Self {
            router,
            max_queries: max_queries.max(1),
        }
    }

    pub fn rule_only() -> Self {
        Self::with_router(Box::new(RuleRouter), 3)
    }

    pub fn route(&self, user_input: &str, prior_one_liner: Option<&str>) -> MemoryRoute {
        let mut route = self.router.route(user_input, prior_one_liner);
        // 両方 true は話題混線の元。知識質問を優先し作業ログ側を落とす。
        if route.work_log && route.knowledge {
            route.work_log = false;
        }
        if route.knowledge && route.queries.is_empty() {
            let q = user_input.trim();
            if !q.is_empty() {
                route.queries.push(q.to_string());
            } else {
                route.knowledge = false;
            }
        }
        if route.queries.len() > self.max_queries {
            route.queries.truncate(self.max_queries);
        }
        route
    }

    /// ターン開始: 分岐 → retrieve → pack。
    pub fn run(
        &self,
        memory: &dyn MemoryBridge,
        config: &MemoryRuntimeConfig,
        user_input: &str,
        prior_one_liner: Option<&str>,
    ) -> PackedRecall {
        let route = self.route(user_input, prior_one_liner);
        self.retrieve(memory, config, &route)
    }

    /// 計画層 `recall` 用: 知識チャネルのみ（route 済みクエリ）。
    pub fn retrieve_knowledge(
        &self,
        memory: &dyn MemoryBridge,
        config: &MemoryRuntimeConfig,
        queries: &[String],
    ) -> Vec<RecalledItem> {
        if !config.search_enabled || queries.is_empty() {
            return vec![];
        }
        search_queries(memory, queries, config.search_top_k)
    }

    fn retrieve(
        &self,
        memory: &dyn MemoryBridge,
        config: &MemoryRuntimeConfig,
        route: &MemoryRoute,
    ) -> PackedRecall {
        let mut packed = PackedRecall {
            route: route.clone(),
            work_log: vec![],
            knowledge: vec![],
        };

        if route.work_log && config.recent_work_enabled {
            match memory.recent_work(config.recent_work_max_entries) {
                Ok(items) => packed.work_log = items,
                Err(err) => eprintln!("[memory.rag] recent_work: {err}"),
            }
        }

        if route.knowledge && config.search_enabled {
            packed.knowledge = search_queries(memory, &route.queries, config.search_top_k);
        }

        packed
    }
}

fn search_queries(
    memory: &dyn MemoryBridge,
    queries: &[String],
    top_k: usize,
) -> Vec<RecalledItem> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for q in queries {
        let q = q.trim();
        if q.is_empty() {
            continue;
        }
        match memory.search(q, top_k) {
            Ok(hits) => {
                for h in hits {
                    let key = h
                        .ref_id
                        .clone()
                        .unwrap_or_else(|| format!("{}::{}", h.title, h.body));
                    if seen.insert(key) {
                        let mut item = h;
                        item.source = RecalledSource::SearchHit;
                        out.push(item);
                    }
                }
            }
            Err(err) => eprintln!("[memory.rag] search({q:?}): {err}"),
        }
    }
    out.truncate(top_k.max(1));
    out
}

/// `PackedRecall` を `PromptBlocks.recalled` へ載せる。
pub fn apply_packed_recall(blocks: &mut PromptBlocks, packed: &PackedRecall, config: &MemoryRuntimeConfig) {
    if !packed.work_log.is_empty() {
        let text = format_recalled_block(
            "recent work",
            &packed.work_log,
            config.recent_work_max_chars,
        );
        blocks.push_recalled(text);
    }
    if !packed.knowledge.is_empty() {
        let text =
            format_recalled_block("search hit", &packed.knowledge, config.search_max_chars);
        blocks.push_recalled(text);
    }
}

/// ターン開始注入（RAG 経由）。戻り値の `route` で session の Previous turns を制御する。
pub fn inject_memory_recalled(
    blocks: &mut PromptBlocks,
    memory: &dyn MemoryBridge,
    config: &MemoryRuntimeConfig,
    rag: &MemoryRag,
    user_input: &str,
    prior_one_liner: Option<&str>,
) -> MemoryRoute {
    let packed = rag.run(memory, config, user_input, prior_one_liner);
    apply_packed_recall(blocks, &packed, config);
    packed.route
}

/// 知識検索のみ（plan `recall`。route はスキップし、呼ぶ側が知識意図と明示）。
pub fn recall_knowledge(
    memory: &dyn MemoryBridge,
    top_k: usize,
    query: &str,
) -> Vec<RecalledItem> {
    let q = query.trim();
    if q.is_empty() {
        return vec![];
    }
    search_queries(memory, &[q.to_string()], top_k.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{DiaryEntry, DiaryPhase, LocalDiaryBridge};

    fn seed_project_diary() -> LocalDiaryBridge {
        let mut memory = LocalDiaryBridge::new();
        memory
            .diary(&DiaryEntry {
                user_input: "このプロジェクトについて説明して".into(),
                summary: "HarnessSeed".into(),
                answer: "ReAct harness の説明".into(),
                phases: vec![DiaryPhase {
                    id: 1,
                    goal: "list".into(),
                    answer: "files".into(),
                }],
            })
            .unwrap();
        memory
    }

    #[test]
    fn rule_router_branches_work_log_vs_knowledge() {
        let r = RuleRouter;
        assert!(r.route("続きやって", None).work_log);
        assert!(!r.route("続きやって", None).knowledge);
        let k = r.route("ファルモってなんじゃ", None);
        assert!(!k.work_log);
        assert!(k.knowledge);
        assert_eq!(k.queries, vec!["ファルモってなんじゃ".to_string()]);
    }

    #[test]
    fn rag_skips_work_log_on_topic_change() {
        let memory = seed_project_diary();
        let rag = MemoryRag::rule_only();
        let packed = rag.run(
            &memory,
            &MemoryRuntimeConfig::default(),
            "ファルモってなんじゃ",
            Some("User: このプロジェクトについて説明して"),
        );
        assert!(!packed.route.work_log);
        assert!(packed.work_log.is_empty());
        assert!(packed.knowledge.is_empty());
    }

    #[test]
    fn rag_loads_work_log_on_continuation() {
        let memory = seed_project_diary();
        let rag = MemoryRag::rule_only();
        let packed = rag.run(
            &memory,
            &MemoryRuntimeConfig::default(),
            "続きやって",
            None,
        );
        assert!(packed.route.work_log);
        assert_eq!(packed.work_log.len(), 1);
        assert!(packed.knowledge.is_empty());
    }

    #[test]
    fn rag_knowledge_search_uses_queries() {
        let mut memory = LocalDiaryBridge::new();
        memory
            .diary(&DiaryEntry {
                user_input: "ファルモ導入".into(),
                summary: "memo".into(),
                answer: "事例メモ".into(),
                phases: vec![],
            })
            .unwrap();
        let rag = MemoryRag::rule_only();
        let packed = rag.run(
            &memory,
            &MemoryRuntimeConfig::default(),
            "ファルモとは",
            None,
        );
        assert!(packed.route.knowledge);
        assert_eq!(packed.knowledge.len(), 1);
        assert!(packed.work_log.is_empty());
    }

    #[test]
    fn parse_route_json_reads_fields() {
        let route = parse_route_json(
            r#"{"work_log":false,"knowledge":true,"queries":["ファルモ","Falmo"]}"#,
            3,
        )
        .unwrap();
        assert!(!route.work_log);
        assert!(route.knowledge);
        assert_eq!(route.queries.len(), 2);
    }

    struct BothTrueRouter;

    impl MemoryRouter for BothTrueRouter {
        fn route(&self, user_input: &str, _: Option<&str>) -> MemoryRoute {
            MemoryRoute {
                work_log: true,
                knowledge: true,
                queries: vec![user_input.trim().to_string()],
            }
        }
    }

    #[test]
    fn guard_drops_work_log_when_both_channels_on() {
        let rag = MemoryRag::with_router(Box::new(BothTrueRouter), 3);
        let route = rag.route("このプロジェクトについて説明して", Some("User: ファルモ"));
        assert!(!route.work_log, "work_log must yield to knowledge");
        assert!(route.knowledge);
        assert!(!route.queries.is_empty());
    }
}
