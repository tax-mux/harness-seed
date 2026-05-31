//! 参照情報 — Harness 内部 JSON の `references` ノード。

use serde::{Deserialize, Serialize};

/// 参照情報（メール）の種別（受信 / 送信待ち / 送信済み）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessMailRefKind {
    Inbox,
    OutgoingPending,
    OutgoingSent,
}

impl HarnessMailRefKind {
    pub fn label_ja(self) -> &'static str {
        match self {
            Self::Inbox => "受信メール",
            Self::OutgoingPending => "送信待ちメール",
            Self::OutgoingSent => "送信済みメール",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim() {
            "inbox" => Self::Inbox,
            "outgoing_pending" => Self::OutgoingPending,
            "outgoing_sent" => Self::OutgoingSent,
            _ => Self::Inbox,
        }
    }
}

/// Harness が保持する参照文書（メール等）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessReference {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub kind: HarnessMailRefKind,
    pub uid: u64,
    pub subject: String,
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub body: String,
}

impl HarnessReference {
    pub fn format_block(&self) -> String {
        let mut out = format!(
            "種別: {}\nUID: {}\n送信者: {}\n",
            self.kind.label_ja(),
            self.uid,
            self.from
        );
        if let Some(ref to) = self.to {
            if !to.trim().is_empty() {
                out.push_str(&format!("宛先: {to}\n"));
            }
        }
        out.push_str(&format!("件名: {}\n本文:\n", self.subject));
        let body = self.body.trim();
        if body.is_empty() {
            out.push_str("(本文なし)");
        } else {
            out.push_str(body);
        }
        out
    }
}

/// 複数参照を Planner 固定ゾーン（参照情報）向けテキストにする。
pub fn format_references_for_prompt(refs: &[HarnessReference]) -> String {
    if refs.is_empty() {
        return String::new();
    }
    let blocks: Vec<String> = refs.iter().map(HarnessReference::format_block).collect();
    format!("【参照情報】\n{}", blocks.join("\n\n---\n\n"))
}
