//! 外部メモリ参照ブリッジ（計画層・実行層への recalled 注入）。
//!
//! **local（プロセス内 diary）は外部バックエンドで置き換えない。**
//! `config.memory.local` + `config.memory.backends` でレイヤを積み、
//! 固有設定は `config.memory.providers.<name>` に置く。
//!
//! ターン開始の分岐・検索語生成は [`rag`]（アダプタ手前の記憶 RAG）。
//! アダプタは `recent_work` / `search` / `diary` の I/O だけを担う。
//!
//! 新しいバックエンドを足す手順:
//! 1. `MemoryBridge` を実装（別クレート推奨）
//! 2. [`factory::build_memory_bridge`] の `build_backend` に名前を登録
//! 3. `backends` に名前を足し `providers.<name>` に設定を書く

mod factory;
mod layered;
mod rag;
#[cfg(feature = "mempalace")]
mod mempalace;

use std::fmt;

use crate::plan::PlanArtifact;

pub use factory::{
    build_memory_bridge, deserialize_provider_options, provider_options, resolve_memory_layers,
    MemoryLayerPlan, PROVIDER_LOCAL, PROVIDER_MEMPALACE, PROVIDER_NOOP,
};
pub use layered::LayeredMemoryBridge;
#[cfg(feature = "mempalace")]
pub use mempalace::MempalaceBridge;
pub use rag::{
    apply_packed_recall, inject_memory_recalled, recall_knowledge, LlmRouter, MemoryRag,
    MemoryRoute, MemoryRouter, PackedRecall, RuleRouter,
};

/// recalled 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecalledItem {
    pub title: String,
    pub body: String,
    pub source: RecalledSource,
    pub ref_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecalledSource {
    RecentWork,
    SearchHit,
}

/// ターン終了時に diary へ書くエントリ。
#[derive(Debug, Clone)]
pub struct DiaryEntry {
    pub user_input: String,
    pub summary: String,
    pub answer: String,
    pub phases: Vec<DiaryPhase>,
}

#[derive(Debug, Clone)]
pub struct DiaryPhase {
    pub id: u32,
    pub goal: String,
    pub answer: String,
}

#[derive(Debug)]
pub enum MemoryError {
    Backend(String),
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(msg) => write!(f, "memory backend: {msg}"),
        }
    }
}

impl std::error::Error for MemoryError {}

/// 外部メモリ（読み取り中心。diary のみ書き込み）。
pub trait MemoryBridge: Send {
    /// 直近の作業状態（作業ログチャネル。呼び元が必要と判断したときだけ）。
    fn recent_work(&self, max_entries: usize) -> Result<Vec<RecalledItem>, MemoryError>;

    /// 知識検索（知識チャネル。呼び元がクエリを決めてから）。
    fn search(&self, query: &str, top_k: usize) -> Result<Vec<RecalledItem>, MemoryError>;

    /// ターン終了時にタスク単位の要約を書き込む。
    fn diary(&mut self, entry: &DiaryEntry) -> Result<(), MemoryError>;
}

/// 既定。何もしない（既存回帰なし）。
#[derive(Debug, Default)]
pub struct NoopBridge;

impl MemoryBridge for NoopBridge {
    fn recent_work(&self, _max_entries: usize) -> Result<Vec<RecalledItem>, MemoryError> {
        Ok(vec![])
    }

    fn search(&self, _query: &str, _top_k: usize) -> Result<Vec<RecalledItem>, MemoryError> {
        Ok(vec![])
    }

    fn diary(&mut self, _entry: &DiaryEntry) -> Result<(), MemoryError> {
        Ok(())
    }
}

/// プロセス内 diary（`memory.provider: "local"`）。
#[derive(Debug, Default)]
pub struct LocalDiaryBridge {
    entries: Vec<DiaryEntry>,
    max_stored: usize,
}

impl LocalDiaryBridge {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            max_stored: 32,
        }
    }

    pub fn with_capacity(max_stored: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_stored: max_stored.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn entry_to_item(&self, entry: &DiaryEntry, index: usize) -> RecalledItem {
        let mut body = String::new();
        body.push_str(&format!("User: {}\n", entry.user_input));
        if !entry.summary.is_empty() {
            body.push_str(&format!("Summary: {}\n", entry.summary));
        }
        if entry.phases.is_empty() {
            body.push_str(&format!("Answer: {}\n", entry.answer));
        } else {
            for p in &entry.phases {
                body.push_str(&format!(
                    "- Phase {}: {}\n  {}\n",
                    p.id, p.goal, p.answer
                ));
            }
        }
        RecalledItem {
            title: truncate_chars(&entry.user_input, 80),
            body,
            source: RecalledSource::RecentWork,
            ref_id: Some(format!("diary#{index}")),
        }
    }
}

impl MemoryBridge for LocalDiaryBridge {
    fn recent_work(&self, max_entries: usize) -> Result<Vec<RecalledItem>, MemoryError> {
        let max_entries = max_entries.max(1);
        let start = self.entries.len().saturating_sub(max_entries);
        let items = self.entries[start..]
            .iter()
            .enumerate()
            .map(|(i, e)| self.entry_to_item(e, start + i + 1))
            .collect();
        Ok(items)
    }

    fn search(&self, query: &str, top_k: usize) -> Result<Vec<RecalledItem>, MemoryError> {
        let q = query.trim().to_lowercase();
        if q.is_empty() || top_k == 0 {
            return Ok(vec![]);
        }
        let mut hits = Vec::new();
        for (i, entry) in self.entries.iter().enumerate().rev() {
            let hay = format!(
                "{} {} {}",
                entry.user_input, entry.summary, entry.answer
            )
            .to_lowercase();
            if text_matches_query(&hay, &q) {
                let mut item = self.entry_to_item(entry, i + 1);
                item.source = RecalledSource::SearchHit;
                hits.push(item);
                if hits.len() >= top_k {
                    break;
                }
            }
        }
        Ok(hits)
    }

    fn diary(&mut self, entry: &DiaryEntry) -> Result<(), MemoryError> {
        self.entries.push(entry.clone());
        while self.entries.len() > self.max_stored {
            self.entries.remove(0);
        }
        Ok(())
    }
}

/// 実行時の注入設定（`config.json` の `memory`）。
#[derive(Debug, Clone)]
pub struct MemoryRuntimeConfig {
    pub recent_work_enabled: bool,
    pub recent_work_max_entries: usize,
    pub recent_work_max_chars: usize,
    pub search_enabled: bool,
    pub search_top_k: usize,
    pub search_max_chars: usize,
    /// 計画層 `recall` ステップの上限（0 で無効）。
    pub recall_max_rounds: usize,
    /// RAG ルータ: `"rule"` | `"llm"`。
    pub rag_router: String,
    /// 知識検索クエリの上限。
    pub rag_max_queries: usize,
}

impl Default for MemoryRuntimeConfig {
    fn default() -> Self {
        Self {
            recent_work_enabled: true,
            recent_work_max_entries: 3,
            recent_work_max_chars: 800,
            search_enabled: true,
            search_top_k: 5,
            search_max_chars: 3200,
            recall_max_rounds: 2,
            // LLM があれば使う。コネクタ無し・失敗時は RuleRouter。
            rag_router: "llm".into(),
            rag_max_queries: 3,
        }
    }
}

pub fn format_recalled_block(label: &str, items: &[RecalledItem], max_chars: usize) -> String {
    let mut out = format!("[{label}]\n");
    for (i, item) in items.iter().enumerate() {
        let n = i + 1;
        let ref_s = item
            .ref_id
            .as_deref()
            .map(|r| format!(" (ref: {r})"))
            .unwrap_or_default();
        out.push_str(&format!("- ({n}) {}{ref_s}\n", item.title));
        out.push_str(&item.body);
        if !item.body.ends_with('\n') {
            out.push('\n');
        }
    }
    truncate_chars(&out, max_chars.max(80))
}

/// `PlanArtifact` があるときの簡易 diary（テスト用）。
pub fn diary_from_plan(user_input: &str, plan: &PlanArtifact, answer: &str) -> DiaryEntry {
    DiaryEntry {
        user_input: user_input.to_string(),
        summary: plan.summary.clone(),
        answer: answer.to_string(),
        phases: plan
            .subtasks
            .iter()
            .map(|s| DiaryPhase {
                id: s.id,
                goal: s.goal.clone(),
                answer: String::new(),
            })
            .collect(),
    }
}

/// `MemoryRuntimeConfig` と任意の LLM コネクタから [`MemoryRag`] を組み立てる。
pub fn build_memory_rag(
    config: &MemoryRuntimeConfig,
    llm: Option<crate::llm::LlmConnectorKind>,
) -> MemoryRag {
    let max_q = config.rag_max_queries.max(1);
    let want_llm = config.rag_router.eq_ignore_ascii_case("llm");
    match (want_llm, llm) {
        (true, Some(connector)) => MemoryRag::with_router(
            Box::new(LlmRouter::new(connector, RuleRouter, max_q)),
            max_q,
        ),
        _ => MemoryRag::with_router(Box::new(RuleRouter), max_q),
    }
}

fn text_matches_query(hay: &str, query: &str) -> bool {
    crate::text_match::text_matches_query(hay, query)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let snippet: String = text.chars().take(max_chars).collect();
    format!("{snippet}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PlanArtifact;

    #[test]
    fn noop_returns_empty() {
        let mut b = NoopBridge;
        assert!(b.recent_work(3).unwrap().is_empty());
        assert!(b.search("x", 5).unwrap().is_empty());
        b.diary(&DiaryEntry {
            user_input: "a".into(),
            summary: "s".into(),
            answer: "ans".into(),
            phases: vec![],
        })
        .unwrap();
    }

    #[test]
    fn local_diary_recent_work_and_search() {
        let mut b = LocalDiaryBridge::new();
        b.diary(&DiaryEntry {
            user_input: "提案資料を書く".into(),
            summary: "draft".into(),
            answer: "途中まで書いた".into(),
            phases: vec![DiaryPhase {
                id: 1,
                goal: "outline".into(),
                answer: "見出し作成".into(),
            }],
        })
        .unwrap();
        let recent = b.recent_work(3).unwrap();
        assert_eq!(recent.len(), 1);
        assert!(recent[0].body.contains("提案資料"));
        assert_eq!(recent[0].source, RecalledSource::RecentWork);

        let hits = b.search("提案", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source, RecalledSource::SearchHit);
        assert!(b.search("存在しない語", 5).unwrap().is_empty());
    }

    #[test]
    fn inject_pushes_work_log_on_continuation() {
        let mut memory = LocalDiaryBridge::new();
        memory
            .diary(&DiaryEntry {
                user_input: "前回の作業".into(),
                summary: "done".into(),
                answer: "完了".into(),
                phases: vec![],
            })
            .unwrap();
        let mut blocks = crate::context::PromptBlocks::new();
        blocks.push_recalled("host note");
        let rag = MemoryRag::rule_only();
        let route = inject_memory_recalled(
            &mut blocks,
            &memory,
            &MemoryRuntimeConfig::default(),
            &rag,
            "続きやって",
            None,
        );
        assert!(route.work_log);
        assert!(!route.knowledge);
        assert!(blocks.recalled.iter().any(|c| c.contains("[recent work]")));
        assert!(blocks.recalled.iter().any(|c| c.contains("host note")));
        assert_eq!(
            blocks
                .recalled
                .iter()
                .filter(|c| c.contains("[search hit]"))
                .count(),
            0
        );
    }

    #[test]
    fn inject_knowledge_when_not_continuation() {
        let mut memory = LocalDiaryBridge::new();
        memory
            .diary(&DiaryEntry {
                user_input: "ファルモ導入".into(),
                summary: "memo".into(),
                answer: "事例メモ".into(),
                phases: vec![],
            })
            .unwrap();
        let mut blocks = crate::context::PromptBlocks::new();
        let rag = MemoryRag::rule_only();
        let route = inject_memory_recalled(
            &mut blocks,
            &memory,
            &MemoryRuntimeConfig::default(),
            &rag,
            "ファルモとは",
            None,
        );
        assert!(!route.work_log);
        assert!(route.knowledge);
        assert!(blocks.recalled.iter().any(|c| c.contains("[search hit]")));
        assert!(blocks.recalled.iter().all(|c| !c.contains("[recent work]")));
    }

    #[test]
    fn inject_skips_work_log_on_topic_change() {
        let mut memory = LocalDiaryBridge::new();
        memory
            .diary(&DiaryEntry {
                user_input: "このプロジェクトについて説明して".into(),
                summary: "HarnessSeed".into(),
                answer: "ReAct harness の説明".into(),
                phases: vec![],
            })
            .unwrap();
        let mut blocks = crate::context::PromptBlocks::new();
        let rag = MemoryRag::rule_only();
        let route = inject_memory_recalled(
            &mut blocks,
            &memory,
            &MemoryRuntimeConfig::default(),
            &rag,
            "ファルモってなんじゃ",
            Some("User: このプロジェクトについて説明して"),
        );
        assert!(!route.work_log);
        assert!(
            blocks
                .recalled
                .iter()
                .all(|c| !c.contains("[recent work]")),
            "topic change must not inject work log"
        );
        assert!(
            blocks
                .recalled
                .iter()
                .all(|c| !c.contains("[search hit]")),
            "unrelated knowledge must not false-hit"
        );
    }

    #[test]
    fn diary_from_plan_helper() {
        let plan = PlanArtifact::single_subtask("do");
        let e = diary_from_plan("u", &plan, "a");
        assert_eq!(e.phases.len(), 1);
    }
}
