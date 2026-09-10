//! 外側の推進ループ — 計画フェーズを順次実行し、要約を `recalled` に載せてロングコンテキストを分割する。

use std::collections::HashSet;

use crate::action::TurnTrace;
use crate::context::PromptBlocks;
use crate::plan::{PlanArtifact, Subtask, EVIDENCE_ORIENTED_DONE_WHEN};
use serde_json::json;

/// 判定前に欲しがる「中身のある」成功ツール observation の既定下限。
pub const MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT: usize = 3;

/// 浅い列挙だけでは証拠に数えないツール以外＝実質証拠として数えるツール。
pub const SUBSTANTIVE_EVIDENCE_TOOLS: &[&str] = &[
    "read_file",
    "grep",
    "web_search",
    "run_cmd",
    "write_file",
];

/// 後方互換エイリアス（旧名）。
pub const MIN_OK_TOOL_OBSERVATIONS_BEFORE_JUDGMENT: usize =
    MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT;

/// 成功したツール observation 数（失敗は除外。浅い list も含む）。
pub fn count_ok_tool_observations(trace: &TurnTrace) -> usize {
    trace.observations.iter().filter(|o| o.ok).count()
}

/// 中身のある成功ツール observation 数（`list_dir` 等の浅い列挙は除外）。
pub fn count_substantive_ok_observations(trace: &TurnTrace) -> usize {
    let mut n = 0usize;
    for (action, obs) in trace.actions.iter().zip(trace.observations.iter()) {
        if obs.ok && is_substantive_evidence_tool(&action.tool) {
            n += 1;
        }
    }
    n
}

pub fn is_substantive_evidence_tool(name: &str) -> bool {
    SUBSTANTIVE_EVIDENCE_TOOLS
        .iter()
        .any(|t| name.eq_ignore_ascii_case(t))
}

pub fn prior_evidence_is_thin(substantive_ok: usize, min_substantive_ok: usize) -> bool {
    substantive_ok < min_substantive_ok.max(1)
}

/// 先行証拠が薄いときに差し込む自由記述サブタスク。
pub fn evidence_deepening_subtask(id: u32) -> Subtask {
    Subtask {
        id,
        task: None,
        params: json!({}),
        goal: "Prior phase evidence is thin on substantive tools (read/grep/search/run). \
Gather more concrete evidence — prefer read_file, grep, or web_search over repeated list_dir. \
Cite specific paths and findings."
            .into(),
        done_when: EVIDENCE_ORIENTED_DONE_WHEN.into(),
        depends_on: vec![],
    }
}

/// 推進ループの設定（`config.json` の `react.advance`）。
#[derive(Debug, Clone)]
pub struct AdvanceConfig {
    /// 有効時は `run_turn` が計画 → フェーズ逐次実行（`two_phase` より優先）。
    pub enabled: bool,
    /// 1 リクエストあたりの最大フェーズ数（計画サブタスクの上限）。
    pub max_phases: usize,
    /// フェーズ間で `SessionMemory` をクリアする（先頭フェーズは保持）。
    pub clear_session_each_phase: bool,
    /// フェーズ要約を `recalled` に載せる最大文字数（1 フェーズあたり）。
    pub max_note_chars: usize,
    /// 各フェーズ開始を stdout に表示する。
    pub show_phases: bool,
    /// 判定前に必要な実質証拠（read/grep 等）成功 observation 数。
    pub min_substantive_obs: usize,
    /// 最終回答のパス引用を先行 Paths と照合し、無いものを未検証注記する。
    pub citation_check: bool,
}

impl Default for AdvanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_phases: 8,
            clear_session_each_phase: true,
            max_note_chars: 1500,
            show_phases: true,
            min_substantive_obs: MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT,
            citation_check: true,
        }
    }
}

/// 完了フェーズの記録（次フェーズへ `recalled` 注入）。
#[derive(Debug, Clone, Default)]
pub struct AdvanceProgress {
    pub mission: String,
    pub plan_summary: String,
    pub steps: Vec<AdvancePhaseNote>,
}

/// 1 フェーズ分の構造化メモ（全文 answer の切り捨てだけに頼らない）。
#[derive(Debug, Clone)]
pub struct AdvancePhaseNote {
    pub id: u32,
    pub goal: String,
    /// フェーズの生回答（参照用。recalled には構造化側を優先）。
    pub answer: String,
    /// ツール引数や本文から拾ったパス・ファイル名。
    pub paths: Vec<String>,
    /// 短い主張・所見（箇条書き等から抽出）。
    pub claims: Vec<String>,
    /// 未解決・未検証として残した点。
    pub open_questions: Vec<String>,
    /// 成功したツール名（重複なし・出現順）。
    pub tools_ok: Vec<String>,
}

impl AdvancePhaseNote {
    /// 回答文だけからノートを作る（trace 無しの replan メモなど）。
    pub fn from_answer(id: u32, goal: impl Into<String>, answer: impl Into<String>) -> Self {
        build_phase_note(id, goal, answer, None)
    }

    /// Recalled / 合成 evidence 用の構造化テキスト（予算内）。
    pub fn format_structured(&self, max_chars: usize) -> String {
        let max_chars = max_chars.max(120);
        let mut out = String::new();
        out.push_str(&format!("Goal: {}\n", self.goal));
        if !self.paths.is_empty() {
            out.push_str("Paths:\n");
            for p in &self.paths {
                out.push_str(&format!("- {p}\n"));
            }
        }
        if !self.tools_ok.is_empty() {
            out.push_str(&format!("Tools: {}\n", self.tools_ok.join(" → ")));
        }
        if !self.claims.is_empty() {
            out.push_str("Claims:\n");
            for c in &self.claims {
                out.push_str(&format!("- {c}\n"));
            }
        }
        if !self.open_questions.is_empty() {
            out.push_str("Open questions:\n");
            for q in &self.open_questions {
                out.push_str(&format!("- {q}\n"));
            }
        }
        // 構造化が空に近いときだけ answer 要約を足す
        if self.paths.is_empty() && self.claims.is_empty() && self.open_questions.is_empty() {
            out.push_str("Result:\n");
            out.push_str(&truncate_note(&self.answer, max_chars.saturating_sub(out.chars().count())));
            out.push('\n');
        } else if out.chars().count() < max_chars.saturating_mul(2) / 3 {
            let remain = max_chars.saturating_sub(out.chars().count()).saturating_sub(24);
            if remain > 80 && !self.answer.trim().is_empty() {
                out.push_str("Answer excerpt:\n");
                out.push_str(&truncate_note(&self.answer, remain));
                out.push('\n');
            }
        }
        truncate_note(&out, max_chars)
    }
}

/// 回答と任意の trace から構造化フェーズノートを組み立てる（機械抽出・ドメイン非依存）。
pub fn build_phase_note(
    id: u32,
    goal: impl Into<String>,
    answer: impl Into<String>,
    trace: Option<&TurnTrace>,
) -> AdvancePhaseNote {
    let goal = goal.into();
    let answer = answer.into();
    let mut paths = Vec::new();
    let mut tools_ok = Vec::new();
    if let Some(trace) = trace {
        for action in &trace.actions {
            if let Some(p) = action.args.get("path").and_then(|v| v.as_str()) {
                push_unique(&mut paths, p.trim());
            }
        }
        for (action, obs) in trace.actions.iter().zip(trace.observations.iter()) {
            if obs.ok {
                push_unique(&mut tools_ok, action.tool.as_str());
            }
        }
    }
    for p in extract_path_like_tokens(&answer) {
        push_unique(&mut paths, &p);
    }
    let claims = extract_claims(&answer, 8, 160);
    let open_questions = extract_open_questions(&answer, 6, 160);
    AdvancePhaseNote {
        id,
        goal,
        answer,
        paths,
        claims,
        open_questions,
        tools_ok,
    }
}

fn push_unique(out: &mut Vec<String>, item: &str) {
    let item = item.trim();
    if item.is_empty() || item.len() > 240 {
        return;
    }
    if !out.iter().any(|e| e == item) {
        out.push(item.to_string());
    }
}

fn extract_path_like_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}' | '、' | '。')) {
        let t = raw
            .trim()
            .trim_matches(|c: char| matches!(c, '*' | '#' | ':' | '：' | '.' | '!' | '?'));
        if t.is_empty() || t.chars().count() > 200 {
            continue;
        }
        let looks_path = t.contains('/')
            || [
                ".rs", ".md", ".toml", ".json", ".txt", ".html", ".yaml", ".yml", ".lock",
            ]
            .iter()
            .any(|ext| t.ends_with(ext));
        if looks_path {
            push_unique(&mut out, t);
        }
    }
    out
}

fn strip_bullet_prefix(line: &str) -> &str {
    let mut s = line.trim();
    for prefix in ["- ", "* ", "・", "– ", "— "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim();
        }
    }
    // numbered: "1. " / "1) "
    if let Some((head, rest)) = s.split_once(['.', ')', '、', ':']) {
        if head.trim().chars().all(|c| c.is_ascii_digit()) && head.len() <= 3 {
            return rest.trim();
        }
    }
    s
}

fn extract_claims(text: &str, max_items: usize, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if out.len() >= max_items {
            break;
        }
        let body = strip_bullet_prefix(line);
        if body.chars().count() < 12 {
            continue;
        }
        if looks_like_open_question(body) {
            continue;
        }
        out.push(truncate_note(body, max_chars));
    }
    if out.is_empty() {
        let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if compact.chars().count() >= 12 {
            out.push(truncate_note(&compact, max_chars));
        }
    }
    out
}

fn looks_like_open_question(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.contains('?')
        || text.contains('？')
        || lower.contains("unverified")
        || lower.contains("unclear")
        || lower.contains("unknown")
        || lower.contains("not sure")
        || lower.contains("todo")
        || text.contains("未確認")
        || text.contains("未検証")
        || text.contains("不明")
        || text.contains("可能性")
}

fn extract_open_questions(text: &str, max_items: usize, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if out.len() >= max_items {
            break;
        }
        let body = strip_bullet_prefix(line);
        if body.chars().count() < 8 {
            continue;
        }
        if looks_like_open_question(body) {
            out.push(truncate_note(body, max_chars));
        }
    }
    out
}

/// 先行フェーズの構造化 Paths（および本文のパス風トークン）を証拠集合にする。
pub fn evidence_paths_from_notes(notes: &[AdvancePhaseNote]) -> HashSet<String> {
    let mut set = HashSet::new();
    for note in notes {
        for p in &note.paths {
            set.insert(p.clone());
        }
        for p in extract_path_like_tokens(&note.answer) {
            set.insert(p);
        }
    }
    set
}

/// フェーズ carry 文字列（合成 evidence）から Paths を集める。
pub fn evidence_paths_from_texts<'a, I>(texts: I) -> HashSet<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut set = HashSet::new();
    for text in texts {
        for p in extract_path_like_tokens(text) {
            set.insert(p);
        }
    }
    set
}

pub fn path_supported_by_evidence(cited: &str, evidence: &HashSet<String>) -> bool {
    let cited = cited.trim();
    if cited.is_empty() {
        return false;
    }
    evidence.iter().any(|e| {
        e == cited
            || e.ends_with(cited)
            || cited.ends_with(e.as_str())
            || e.contains(cited)
            || cited.contains(e.as_str())
    })
}

/// 回答中のパス風参照のうち、証拠 Paths に無いものを列挙する。
pub fn unverified_cited_paths(answer: &str, evidence: &HashSet<String>) -> Vec<String> {
    let mut out = Vec::new();
    for cited in extract_path_like_tokens(answer) {
        if path_supported_by_evidence(&cited, evidence) {
            continue;
        }
        if !out.iter().any(|e| e == &cited) {
            out.push(cited);
        }
    }
    out
}

/// 証拠に無いパス引用を未検証として注記する（機械ゲート・ドメイン非依存）。
pub fn apply_citation_gate(answer: &str, evidence: &HashSet<String>) -> String {
    let unverified = unverified_cited_paths(answer, evidence);
    if unverified.is_empty() {
        return answer.to_string();
    }
    let mut out = answer.trim_end().to_string();
    out.push_str(
        "\n\n## Citation check\n\
The following path-like references were not found in prior-phase evidence Paths; \
treat them as unverified until re-checked:\n",
    );
    for u in unverified {
        out.push_str(&format!("- `{u}`\n"));
    }
    out
}

impl AdvanceProgress {
    pub fn new(mission: impl Into<String>, plan_summary: impl Into<String>) -> Self {
        Self {
            mission: mission.into(),
            plan_summary: plan_summary.into(),
            steps: Vec::new(),
        }
    }

    pub fn push_note(&mut self, note: AdvancePhaseNote) {
        self.steps.push(note);
    }

    pub fn push(&mut self, id: u32, goal: impl Into<String>, answer: impl Into<String>) {
        self.push_note(AdvancePhaseNote::from_answer(id, goal, answer));
    }

    pub fn evidence_paths(&self) -> HashSet<String> {
        evidence_paths_from_notes(&self.steps)
    }
}

/// 推進ループ 1 フェーズの実行サマリ（`TurnResult.advance_phases` 用）。
#[derive(Debug, Clone)]
pub struct AdvancePhaseSummary {
    pub id: u32,
    pub goal: String,
    pub answer: String,
    pub steps_used: usize,
}

fn truncate_note(text: &str, max_chars: usize) -> String {
    let max_chars = max_chars.max(80);
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let snippet: String = text.chars().take(max_chars).collect();
    format!("{snippet}…")
}

/// 先行フェーズ結果があるときの根拠拘束（判定・列挙・まとめ系で汎用）。
pub fn evidence_grounding_rules() -> &'static str {
    "## Evidence grounding (required)\n\
- Tie every substantive claim to evidence in Recalled / prior phase results \
(prefer Paths, Claims, and Open questions sections when present).\n\
- If a point is not supported by that evidence, label it as an unverified candidate \
or gather more evidence with tools before asserting it.\n\
- Do not answer with generic advice that could apply to any unrelated project \
without citing this turn's evidence.\n"
}

/// 完了フェーズの要約を `recalled` 用テキストにする。
pub fn format_recalled_progress(
    progress: &AdvanceProgress,
    plan: &PlanArtifact,
    max_note_chars: usize,
) -> String {
    let mut out = String::from("## Advance progress (completed phases only)\n\n");
    out.push_str(&format!("Mission: {}\n", progress.mission));
    out.push_str(&format!(
        "Plan summary: {}\n",
        if progress.plan_summary.is_empty() {
            &plan.summary
        } else {
            &progress.plan_summary
        }
    ));
    if progress.steps.is_empty() {
        out.push_str("\n(No prior phases yet.)\n");
        return out;
    }
    out.push('\n');
    for note in &progress.steps {
        out.push_str(&format!(
            "### Phase {} — done\n{}\n",
            note.id,
            note.format_structured(max_note_chars)
        ));
    }
    out.push_str(
        "Use the above as ground truth. Do not redo completed phases unless the current goal requires it.\n\n",
    );
    out.push_str(evidence_grounding_rules());
    out
}

fn format_phase_directive(plan: &PlanArtifact, current: &Subtask, has_prior_phases: bool) -> String {
    let mut out = String::from("## Current phase (execute ONLY this)\n\n");
    out.push_str(&format!(
        "Phase {} / {}\nGoal: {}\nDone when: {}\n\n",
        current.id,
        plan.subtasks.len(),
        current.goal,
        current.done_when
    ));
    if let Some(task) = &current.task {
        out.push_str(&format!("Registered task id: {task}\n"));
    }
    out.push_str(
        "Complete only this phase. Prior phase results are in Recalled context above.\n",
    );
    if has_prior_phases {
        out.push('\n');
        out.push_str(evidence_grounding_rules());
    }
    out
}

/// フェーズ開始前に `PromptBlocks::recalled` を組み立てる（ホスト注入分は保持）。
pub fn prepare_phase_recalled(
    blocks: &mut PromptBlocks,
    base_recalled: &[String],
    progress: &AdvanceProgress,
    plan: &PlanArtifact,
    current: &Subtask,
    config: &AdvanceConfig,
) {
    blocks.clear_recalled();
    for chunk in base_recalled {
        blocks.push_recalled(chunk.as_str());
    }
    let has_prior = !progress.steps.is_empty();
    if has_prior {
        blocks.push_recalled(format_recalled_progress(
            progress,
            plan,
            config.max_note_chars,
        ));
    }
    blocks.push_recalled(format_phase_directive(plan, current, has_prior));
}

/// 推進ループ終了後にホストの `recalled` を復元する。
pub fn restore_base_recalled(blocks: &mut PromptBlocks, base_recalled: &[String]) {
    blocks.clear_recalled();
    for chunk in base_recalled {
        blocks.push_recalled(chunk.as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PlanArtifact;

    #[test]
    fn recalled_progress_lists_prior_phases() {
        let plan = PlanArtifact::single_subtask("mission");
        let mut progress = AdvanceProgress::new("mission", "plan sum");
        progress.push(1, "first goal", "first answer");
        let text = format_recalled_progress(&progress, &plan, 500);
        assert!(text.contains("Phase 1 — done"));
        assert!(text.contains("first answer"));
        assert!(text.contains("Mission: mission"));
    }

    #[test]
    fn second_phase_recalled_contains_first_answer() {
        let plan = PlanArtifact {
            summary: "two steps".into(),
            skip_execution: false,
            subtasks: vec![
                Subtask {
                    id: 1,
                    task: None,
                    params: json!({}),
                    goal: "step one".into(),
                    done_when: "done".into(),
                                    depends_on: vec![],
},
                Subtask {
                    id: 2,
                    task: None,
                    params: json!({}),
                    goal: "step two".into(),
                    done_when: "done".into(),
                                    depends_on: vec![],
},
            ],
            knowledge_sufficient: None,
            user_reply: None,
        };
        let mut progress = AdvanceProgress::new("mission", "two steps");
        progress.push(1, "step one", "answer one");
        let text = format_recalled_progress(&progress, &plan, 500);
        assert!(text.contains("answer one"));
        assert!(text.contains("Phase 1 — done"));
        assert!(text.contains("Evidence grounding"));
        assert!(text.contains("Paths:") || text.contains("Claims:") || text.contains("Goal:"));
    }

    #[test]
    fn prepare_phase_includes_directive() {
        let plan = PlanArtifact::single_subtask("do thing");
        let progress = AdvanceProgress::default();
        let mut blocks = PromptBlocks::new();
        blocks.push_recalled("host note");
        let base = blocks.recalled.clone();
        let st = plan.subtasks[0].clone();
        prepare_phase_recalled(
            &mut blocks,
            &base,
            &progress,
            &plan,
            &st,
            &AdvanceConfig::default(),
        );
        // base was cleared and re-pushed; should have host + directive
        assert!(blocks.recalled.iter().any(|c| c.contains("host note")));
        assert!(blocks.recalled.iter().any(|c| c.contains("Current phase")));
        assert!(!blocks
            .recalled
            .iter()
            .any(|c| c.contains("Evidence grounding")));
    }

    #[test]
    fn prepare_later_phase_includes_evidence_grounding() {
        let plan = PlanArtifact {
            summary: "two".into(),
            skip_execution: false,
            subtasks: vec![
                Subtask {
                    id: 1,
                    task: None,
                    params: json!({}),
                    goal: "gather".into(),
                    done_when: "done".into(),
                    depends_on: vec![],
                },
                Subtask {
                    id: 2,
                    task: None,
                    params: json!({}),
                    goal: "judge".into(),
                    done_when: "done".into(),
                    depends_on: vec![],
                },
            ],
            knowledge_sufficient: None,
            user_reply: None,
        };
        let mut progress = AdvanceProgress::new("mission", "two");
        progress.push(1, "gather", "saw src/lib.rs and a replan bug");
        let mut blocks = PromptBlocks::new();
        let base = Vec::new();
        prepare_phase_recalled(
            &mut blocks,
            &base,
            &progress,
            &plan,
            &plan.subtasks[1],
            &AdvanceConfig::default(),
        );
        let joined = blocks.recalled.join("\n");
        assert!(joined.contains("Evidence grounding"));
        assert!(joined.contains("saw src/lib.rs"));
        assert!(joined.contains("unverified candidate"));
    }

    #[test]
    fn prior_evidence_thinness_threshold() {
        assert!(prior_evidence_is_thin(0, 4));
        assert!(prior_evidence_is_thin(3, 4));
        assert!(!prior_evidence_is_thin(
            MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT,
            MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT
        ));
        assert!(!prior_evidence_is_thin(10, 4));
        let boost = evidence_deepening_subtask(99);
        assert!(boost.goal.contains("thin"));
        assert!(boost.done_when.contains("concrete evidence"));
        assert!(boost.goal.contains("list_dir"));
    }

    #[test]
    fn count_ok_tool_observations_ignores_failures() {
        use crate::action::{Observation, TurnTrace};
        let mut trace = TurnTrace::default();
        trace.push_observation(Observation::success(1, "ok"));
        trace.push_observation(Observation::failure(2, "err"));
        trace.push_observation(Observation::success(3, "ok2"));
        assert_eq!(count_ok_tool_observations(&trace), 2);
    }

    #[test]
    fn count_substantive_ok_observations_skips_list_dir() {
        use crate::action::{Action, Observation, TurnTrace};
        use serde_json::json;
        let mut trace = TurnTrace::default();
        trace.push_action(Action::new(1, "list_dir", json!({ "path": "." })));
        trace.push_observation(Observation::success(1, "a b"));
        trace.push_action(Action::new(2, "list_dir", json!({ "path": "src" })));
        trace.push_observation(Observation::success(2, "lib.rs"));
        trace.push_action(Action::new(3, "read_file", json!({ "path": "src/lib.rs" })));
        trace.push_observation(Observation::success(3, "mod"));
        trace.push_action(Action::new(4, "grep", json!({ "pattern": "Advance" })));
        trace.push_observation(Observation::failure(4, "nope"));
        assert_eq!(count_ok_tool_observations(&trace), 3);
        assert_eq!(count_substantive_ok_observations(&trace), 1);
        assert!(prior_evidence_is_thin(
            count_substantive_ok_observations(&trace),
            MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT
        ));
    }

    #[test]
    fn citation_gate_marks_unsupported_paths() {
        let mut evidence = HashSet::new();
        evidence.insert("src/lib.rs".into());
        evidence.insert("src/advance.rs".into());
        let answer = "See src/lib.rs and invented/feature.rs plus src/config.rs.";
        let gated = apply_citation_gate(answer, &evidence);
        assert!(gated.contains("## Citation check"));
        assert!(gated.contains("`invented/feature.rs`"));
        assert!(gated.contains("`src/config.rs`"));
        let unverified = unverified_cited_paths(answer, &evidence);
        assert!(!unverified.iter().any(|p| p == "src/lib.rs"));
        assert!(unverified.iter().any(|p| p == "invented/feature.rs"));
        assert!(unverified.iter().any(|p| p == "src/config.rs"));
        let clean = apply_citation_gate("Only src/lib.rs matters.", &evidence);
        assert!(!clean.contains("Citation check"));
    }

    #[test]
    fn build_phase_note_extracts_paths_claims_and_open_questions() {
        use crate::action::{Action, Observation, TurnTrace};
        use serde_json::json;
        let mut trace = TurnTrace::default();
        trace.push_action(Action::new(1, "read_file", json!({ "path": "src/lib.rs" })));
        trace.push_observation(Observation::success(1, "pub mod plan;"));
        trace.push_action(Action::new(2, "list_dir", json!({ "path": "doc" })));
        trace.push_observation(Observation::success(2, "ja\nen"));
        let answer = "\
- README.md covers the CLI overview
- src/config.rs parses many fields
- Redis backend support is unverified
- 設定の範囲チェックは未確認
";
        let note = build_phase_note(1, "gather", answer, Some(&trace));
        assert!(note.paths.iter().any(|p| p == "src/lib.rs"));
        assert!(note.paths.iter().any(|p| p == "doc"));
        assert!(note.paths.iter().any(|p| p.contains("README.md")));
        assert!(note.tools_ok.iter().any(|t| t == "read_file"));
        assert!(note.claims.iter().any(|c| c.contains("CLI overview")));
        assert!(note.open_questions.iter().any(|q| q.contains("unverified") || q.contains("未確認")));
        let formatted = note.format_structured(800);
        assert!(formatted.contains("Paths:"));
        assert!(formatted.contains("Claims:"));
        assert!(formatted.contains("Open questions:"));
    }
}
