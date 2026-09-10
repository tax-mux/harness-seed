//! 主張監査サブタスク生成。

/// 主張の否定証拠を一度探す監査サブタスク（ドメイン非依存）。
pub fn claim_falsification_subtask(id: u32) -> crate::plan::Subtask {
    crate::plan::Subtask {
        id,
        task: None,
        params: serde_json::json!({}),
        goal: "Audit prior-phase Claims before concluding. \
For each substantive claim — especially absence claims (does not exist / none / 無い) — \
try once to find contradictory evidence with tools (prefer grep or read_file on cited Paths). \
Label each claim as still supported, falsified (with counter-evidence path/finding), or unverified. \
Do not invent new recommendations in this phase — only stress-test existing claims. \
You must run at least one substantive tool (grep/read_file) before answering."
            .into(),
        done_when: "Each audited claim is labeled supported, falsified, or unverified with a concrete path or tool finding; at least one grep or read_file was used."
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
        goal: "Previous claim audit used no substantive tools. Retry: pick the strongest \
prior-phase Claims (especially absences) and run grep/read_file to seek counter-evidence. \
Label supported / falsified / unverified. No new recommendations."
            .into(),
        done_when:
            "At least one successful grep or read_file observation exists, and claims are labeled."
                .into(),
        depends_on: vec![],
    }
}

/// 主張監査フェーズの結果を後続・合成が優先するための拘束。
pub fn claim_audit_rules() -> &'static str {
    "## Claim audit (required when present)\n\
- If a prior phase audited claims (supported / falsified / unverified), treat falsified claims as rejected.\n\
- Do not repeat falsified claims as facts or high-confidence proposals.\n\
- Prefer still-supported claims; keep unverified items explicitly marked.\n\
- Absence claims (X does not exist / none found) require a tool search in evidence; otherwise treat as unverified.\n"
}
