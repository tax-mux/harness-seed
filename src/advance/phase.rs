//! `AdvancePhaseNote`, `AdvanceProgress`, `format_recalled_progress`, `prepare_phase_recalled`, `restore_base_recalled`.

use crate::action::TurnTrace;
use crate::plan::{PlanArtifact, Subtask};
use std::collections::HashSet;

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
            out.push_str(&truncate_note(
                &self.answer,
                max_chars.saturating_sub(out.chars().count()),
            ));
            out.push('\n');
        } else if out.chars().count() < max_chars.saturating_mul(2) / 3 {
            let remain = max_chars
                .saturating_sub(out.chars().count())
                .saturating_sub(24);
            if remain > 80 && !self.answer.trim().is_empty() {
                out.push_str("Answer excerpt:\n");
                out.push_str(&truncate_note(&self.answer, remain));
                out.push('\n');
            }
        }
        truncate_note(&out, max_chars)
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

pub(super) fn extract_path_like_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '`' | '"'
                    | '\''
                    | ','
                    | ';'
                    | ')'
                    | '('
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '、'
                    | '。'
                    | '（'
                    | '）'
                    | '「'
                    | '」'
            )
    }) {
        let t = raw
            .trim()
            .trim_matches(|c: char| matches!(c, '*' | '#' | ':' | '：' | '.' | '!' | '?' | '・'));
        if is_plausible_path_token(t) {
            push_unique(&mut out, t);
        }
    }
    out
}

/// パス風トークンとして引用照合に載せてよいものか（日本語見出しの誤検知を避ける）。
fn is_plausible_path_token(t: &str) -> bool {
    if t.is_empty() || t.chars().count() > 200 {
        return false;
    }
    let chars: Vec<char> = t.chars().collect();
    let ascii = chars.iter().filter(|c| c.is_ascii()).count();
    // 半分以上が非 ASCII ならパス扱いしない（「CI/CDの整備」等）
    if ascii * 2 < chars.len() {
        return false;
    }
    let path_ok = |c: char| {
        c.is_ascii_alphanumeric() || matches!(c, '/' | '\\' | '.' | '_' | '-' | '*' | '+')
    };
    if !chars.iter().all(|c| path_ok(*c)) {
        return false;
    }
    if t.contains('/') || t.contains('\\') {
        // セグメントが空や記号だけは落とす
        return t.split(['/', '\\']).filter(|s| !s.is_empty()).all(|seg| {
            seg.chars()
                .any(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        });
    }
    [
        ".rs", ".md", ".toml", ".json", ".txt", ".html", ".yaml", ".yml", ".lock",
    ]
    .iter()
    .any(|ext| t.ends_with(ext))
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

/// 完了フェーズの記録（次フェーズへ `recalled` 注入）。
#[derive(Debug, Clone, Default)]
pub struct AdvanceProgress {
    pub mission: String,
    pub plan_summary: String,
    pub steps: Vec<AdvancePhaseNote>,
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

    /// Claims または Paths が先行メモにあれば監査対象あり。
    pub fn has_auditable_claims(&self) -> bool {
        self.steps
            .iter()
            .any(|n| !n.claims.is_empty() || !n.paths.is_empty())
    }
}

/// [`AdvanceProgress::has_auditable_claims`] の関數形。
pub fn prior_has_auditable_claims(progress: &AdvanceProgress) -> bool {
    progress.has_auditable_claims()
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
    out.push('\n');
    out.push_str(advance_audit_rules());
    out
}

fn advance_audit_rules() -> &'static str {
    "## Claim audit (required when present)\n\
    - If a prior phase audited claims (supported / falsified / unverified), treat falsified claims as rejected.\n\
    - Do not repeat falsified claims as facts or high-confidence proposals.\n\
    - Prefer still-supported claims; keep unverified items explicitly marked.\n\
    - Absence claims (X does not exist / none found) require a tool search in evidence; otherwise treat as unverified.\n"
}

fn format_phase_directive(
    plan: &PlanArtifact,
    current: &Subtask,
    has_prior_phases: bool,
) -> String {
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
    out.push_str("Complete only this phase. Prior phase results are in Recalled context above.\n");
    if has_prior_phases {
        out.push('\n');
        out.push_str(evidence_grounding_rules());
        out.push('\n');
        out.push_str(advance_audit_rules());
    }
    out
}

/// フェーズ開始前に `PromptBlocks::recalled` を組み立てる（ホスト注入分は保持）。
pub fn prepare_phase_recalled(
    blocks: &mut crate::context::PromptBlocks,
    base_recalled: &[String],
    progress: &AdvanceProgress,
    plan: &PlanArtifact,
    current: &Subtask,
    config: &crate::AdvanceConfig,
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
pub fn restore_base_recalled(blocks: &mut crate::context::PromptBlocks, base_recalled: &[String]) {
    blocks.clear_recalled();
    for chunk in base_recalled {
        blocks.push_recalled(chunk.as_str());
    }
}

/// 先行フェーズの構造化 Paths（および本文のパス風トークン）を証拠集合にする。
pub fn evidence_paths_from_notes(
    notes: &[crate::advance::phase::AdvancePhaseNote],
) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
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
pub fn evidence_paths_from_texts<'a, I>(texts: I) -> std::collections::HashSet<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut set = std::collections::HashSet::new();
    for text in texts {
        for p in extract_path_like_tokens(text) {
            set.insert(p);
        }
    }
    set
}
