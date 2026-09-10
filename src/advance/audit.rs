//! 主張監査サブタスク生成。

/// 主張の否定証拠を一度探す監査サブタスク（ドメイン非依存）。
pub fn claim_falsification_subtask(id: u32) -> crate::plan::Subtask {
    crate::plan::Subtask {
        id,
        task: None,
        params: serde_json::json!({}),
        goal: "Audit prior-phase Claims before concluding. \
For each substantive claim — especially absence claims (does not exist / none / 無い) — \
try at most twice to find contradictory evidence. Prefer grep/read_file on cited Paths, \
or web_search for news/URL claims. Do NOT keep retrying curl/fetch when stdout is empty \
or the same command fails — label those claims unverified and answer. \
Do not invent new recommendations. Run at least one substantive tool before answering, \
then stop within a few steps."
            .into(),
        done_when: "Each audited claim is labeled supported, falsified, or unverified; \
empty/unreachable fetches are unverified (not retried); at least one substantive tool was used."
            .into(),
        depends_on: vec![],
    }
}

/// 主張監査がツール無しで終わったときの再試行。
pub fn claim_falsification_retry_subtask(id: u32) -> crate::plan::Subtask {
    crate::plan::Subtask {
        id,
        task: None,
        params: serde_json::json!({}),
        goal: "Previous claim audit used no substantive tools. Retry once: pick the strongest \
prior-phase Claims and run grep/read_file or web_search. If a fetch returns empty, label \
unverified and answer — do not loop on curl. Label supported / falsified / unverified. \
No new recommendations."
            .into(),
        done_when:
            "At least one substantive tool observation exists (or empty fetch labeled unverified), and claims are labeled."
                .into(),
        depends_on: vec![],
    }
}

/// 空のツール連打で監査を打ち切ったときの回答本文。
pub const CLAIM_AUDIT_STERILE_ABORT_ANSWER: &str = "\
Claim audit stopped: repeated empty tool output (e.g. empty curl/stdout). \
Remaining claims are labeled **unverified** due to unreachable or empty evidence sources. \
Do not treat them as falsified. No new recommendations.";

/// 主張監査フェーズの結果を後続・合成が優先するための拘束。
pub fn claim_audit_rules() -> &'static str {
    "## Claim audit (required when present)\n\
- If a prior phase audited claims (supported / falsified / unverified), treat falsified claims as rejected.\n\
- Do not repeat falsified claims as facts or high-confidence proposals.\n\
- Prefer still-supported claims; keep unverified items explicitly marked.\n\
- Absence claims (X does not exist / none found) require a tool search in evidence; otherwise treat as unverified.\n\
- Empty/unreachable fetches are unverified — do not keep retrying the same fetch.\n"
}
