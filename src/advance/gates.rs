//! 証拠・パス照合ゲート。

use crate::action::TurnTrace;

/// 証拠に無いパス引用を未検証として注記する（機械ゲート・ドメイン非依存）。
pub fn apply_citation_gate(answer: &str, evidence: &std::collections::HashSet<String>) -> String {
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

/// 不在・欠如を断定している文か（ドメイン非依存の表層パターン）。
pub fn looks_like_absence_claim(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.contains("無い")
        || text.contains("ない")
            && (text.contains("存在") || text.contains("一切") || text.contains("見当"))
        || text.contains("存在しない")
        || text.contains("存在せず")
        || text.contains("見当たらない")
        || text.contains("含まれていない")
        || text.contains("未実装")
        || text.contains("ゼロ")
        || lower.contains("does not exist")
        || lower.contains("do not exist")
        || lower.contains("doesn't exist")
        || lower.contains("not found")
        || lower.contains("no such")
        || lower.contains("there are no")
        || lower.contains("there is no")
        || lower.contains("without any")
        || lower.contains("never implemented")
        || lower.contains("not present")
        || lower.contains("absent")
        || lower.contains("zero tests")
        || (lower.contains("no ")
            && (lower.contains("test") || lower.contains("file") || lower.contains("support")))
        || (lower.contains("none") && lower.contains("found"))
}

/// 回答から不在主張行を抽出する。
pub fn extract_absence_claims(text: &str, max_items: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if out.len() >= max_items {
            break;
        }
        let body = strip_bullet_prefix(line);
        if body.chars().count() < 10 {
            continue;
        }
        if looks_like_absence_claim(body) {
            out.push(truncate_note(body, 200));
        }
    }
    out
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

/// 不在主張から、trace 照合用のアンカー（パス・コード片）を取る。
pub fn absence_claim_anchors(claim: &str) -> Vec<String> {
    let mut anchors = extract_path_like_tokens(claim);
    let mut parts = claim.split('`');
    while let Some(_before) = parts.next() {
        if let Some(code) = parts.next() {
            let code = code.trim();
            if (2..100).contains(&code.chars().count()) {
                push_unique(&mut anchors, code);
            }
        }
    }
    // #[...] 属性風
    let mut rest = claim;
    while let Some(start) = rest.find("#[") {
        let slice = &rest[start..];
        if let Some(end) = slice.find(']') {
            let attr = &slice[..=end];
            if attr.chars().count() <= 80 {
                push_unique(&mut anchors, attr);
            }
            rest = &slice[end + 1..];
        } else {
            break;
        }
    }
    anchors
}

fn extract_path_like_tokens(text: &str) -> Vec<String> {
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

fn is_plausible_path_token(t: &str) -> bool {
    if t.is_empty() || t.chars().count() > 200 {
        return false;
    }
    let chars: Vec<char> = t.chars().collect();
    let ascii = chars.iter().filter(|c| c.is_ascii()).count();
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

fn push_unique(out: &mut Vec<String>, item: &str) {
    let item = item.trim();
    if item.is_empty() || item.len() > 240 {
        return;
    }
    if !out.iter().any(|e| e == item) {
        out.push(item.to_string());
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

pub fn path_supported_by_evidence(
    cited: &str,
    evidence: &std::collections::HashSet<String>,
) -> bool {
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
pub fn unverified_cited_paths(
    answer: &str,
    evidence: &std::collections::HashSet<String>,
) -> Vec<String> {
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

/// 不在主張の機械分類。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbsenceClaimVerdict {
    /// 関連する検索が trace に無い。
    Unverified,
    /// 関連検索があり、出力が空っぽくない（不在断定と矛盾しうる）。
    Contradicted,
}

/// 不在主張を trace と照合する。
pub fn classify_absence_claims(
    answer: &str,
    trace: &TurnTrace,
) -> Vec<(String, AbsenceClaimVerdict)> {
    let mut out = Vec::new();
    for claim in extract_absence_claims(answer, 12) {
        let anchors = absence_claim_anchors(&claim);
        if anchors.is_empty() {
            if count_substantive_ok_observations(trace) == 0
                && count_ok_tool_observations(trace) == 0
            {
                out.push((claim, AbsenceClaimVerdict::Unverified));
            } else {
                out.push((claim, AbsenceClaimVerdict::Unverified));
            }
            continue;
        }
        let mut saw_related = false;
        let mut saw_nonempty = false;
        for a in &anchors {
            if let Some(nonempty) = anchor_matched_in_trace(trace, a) {
                saw_related = true;
                if nonempty {
                    saw_nonempty = true;
                }
            }
        }
        if !saw_related {
            out.push((claim, AbsenceClaimVerdict::Unverified));
        } else if saw_nonempty {
            out.push((claim, AbsenceClaimVerdict::Contradicted));
        }
    }
    out
}

fn observation_looks_nonempty(output: &str) -> bool {
    let t = output.trim();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    if lower.contains("0 matches")
        || lower.contains("no matches")
        || lower.contains("not found")
        || lower.contains("no such file")
        || lower == "[]"
        || lower == "(empty)"
    {
        return false;
    }
    t.chars().count() >= 8
}

fn action_blob(action: &crate::action::Action, obs: &crate::action::Observation) -> String {
    format!("{} {} {}", action.tool, action.args, obs.output)
}

fn anchor_matched_in_trace(trace: &TurnTrace, anchor: &str) -> Option<bool> {
    let anchor_l = anchor.to_ascii_lowercase();
    let mut any = false;
    let mut nonempty = false;
    for (action, obs) in trace.actions.iter().zip(trace.observations.iter()) {
        if !obs.ok {
            continue;
        }
        if !is_substantive_evidence_tool(&action.tool)
            && !action.tool.eq_ignore_ascii_case("list_dir")
        {
            continue;
        }
        let blob = action_blob(action, obs).to_ascii_lowercase();
        if blob.contains(&anchor_l) {
            any = true;
            if observation_looks_nonempty(&obs.output) {
                nonempty = true;
            }
        }
    }
    if any {
        Some(nonempty)
    } else {
        None
    }
}

fn is_substantive_evidence_tool(name: &str) -> bool {
    crate::advance::evidence::SUBSTANTIVE_EVIDENCE_TOOLS
        .iter()
        .any(|t| name.eq_ignore_ascii_case(t))
}

fn count_ok_tool_observations(trace: &TurnTrace) -> usize {
    trace.observations.iter().filter(|o| o.ok).count()
}

fn count_substantive_ok_observations(trace: &TurnTrace) -> usize {
    let mut n = 0usize;
    for (action, obs) in trace.actions.iter().zip(trace.observations.iter()) {
        if obs.ok && is_substantive_evidence_tool(&action.tool) {
            n += 1;
        }
    }
    n
}

/// 不在主張ゲート（機械的注記）。
pub fn apply_absence_gate(answer: &str, trace: &TurnTrace) -> String {
    let classified = classify_absence_claims(answer, trace);
    let unverified: Vec<_> = classified
        .iter()
        .filter(|(_, v)| *v == AbsenceClaimVerdict::Unverified)
        .map(|(c, _)| c.clone())
        .collect();
    let contradicted: Vec<_> = classified
        .iter()
        .filter(|(_, v)| *v == AbsenceClaimVerdict::Contradicted)
        .map(|(c, _)| c.clone())
        .collect();
    if unverified.is_empty() && contradicted.is_empty() {
        return answer.to_string();
    }
    let mut out = answer.trim_end().to_string();
    if !unverified.is_empty() {
        out.push_str(
            "\n\n## Unverified absence\n\
            The following absence claims have no matching search/read in this turn's tool trace; \
            treat them as unverified:\n",
        );
        for u in unverified {
            out.push_str(&format!("- {u}\n"));
        }
    }
    if !contradicted.is_empty() {
        out.push_str(
            "\n\n## Contradicted absence\n\
            The following absence claims conflict with non-empty tool observations that mention the same anchors; \
            do not treat them as established facts:\n",
        );
        for c in contradicted {
            out.push_str(&format!("- {c}\n"));
        }
    }
    out
}
