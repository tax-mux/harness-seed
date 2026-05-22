//! プロンプト内セクションのサイズ分解とターミナル用カラーマップ表示。

use crate::context_metrics::estimated_tokens_from_chars;
use crate::llm::ChatMessage;

/// プロンプト内の論理ブロック（[context-memory-mapping.md](../doc/context-memory-mapping.md) 対応）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextSectionKind {
    SystemCore,
    ToolCatalog,
    Rules,
    Recalled,
    SystemExtra,
    PreviousTurns,
    UserInput,
    TurnTrace,
    NextStepCue,
    Other,
}

impl ContextSectionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::SystemCore => "react_core",
            Self::ToolCatalog => "tool_catalog",
            Self::Rules => "rules",
            Self::Recalled => "recalled",
            Self::SystemExtra => "system_extra",
            Self::PreviousTurns => "previous_turns",
            Self::UserInput => "user_input",
            Self::TurnTrace => "turn_trace",
            Self::NextStepCue => "next_step",
            Self::Other => "other",
        }
    }
}

/// 1 セクションのサイズ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSection {
    pub kind: ContextSectionKind,
    pub chars: usize,
}

impl ContextSection {
    pub fn estimated_tokens(&self) -> u32 {
        estimated_tokens_from_chars(self.chars)
    }
}

/// `format_messages_body` 形式または system/user 本文からセクションを推定する。
pub fn analyze_prompt_body(body: &str) -> Vec<ContextSection> {
    let mut out = Vec::new();
    let mut pos = 0usize;

    while pos < body.len() {
        let rest = &body[pos..];
        let Some((role, prefix_len)) = parse_role_prefix(rest) else {
            if out.is_empty() && !body.trim().is_empty() {
                out.extend(analyze_system_content(body.trim()));
            }
            break;
        };
        let content_start = pos + prefix_len;
        let content_rest = &body[content_start..];
        let rel_end = find_next_role_line(content_rest).unwrap_or(content_rest.len());
        let content = body[content_start..content_start + rel_end].trim_end();
        pos = content_start + rel_end;

        match role {
            "system" => out.extend(analyze_system_content(content)),
            "user" => out.extend(analyze_user_content(content)),
            _ => push_section(&mut out, ContextSectionKind::Other, content.chars().count()),
        }
    }

    merge_adjacent(out)
}

fn parse_role_prefix(s: &str) -> Option<(&'static str, usize)> {
    for role in ["system", "user", "assistant"] {
        let prefix = format!("{role}:");
        if s.starts_with(&prefix) {
            return Some((role, prefix.len()));
        }
    }
    None
}

fn find_next_role_line(s: &str) -> Option<usize> {
    for (i, _) in s.match_indices('\n') {
        let tail = &s[i + 1..];
        if tail.starts_with("user:")
            || tail.starts_with("system:")
            || tail.starts_with("assistant:")
        {
            return Some(i + 1);
        }
    }
    None
}

/// 最新の LLM 呼び出しメッセージからセクション分解。
pub fn analyze_messages(messages: &[ChatMessage]) -> Vec<ContextSection> {
    let mut out = Vec::new();
    for msg in messages {
        match msg.role.as_str() {
            "system" => out.extend(analyze_system_content(&msg.content)),
            "user" => out.extend(analyze_user_content(&msg.content)),
            _ => push_section(&mut out, ContextSectionKind::Other, msg.content.chars().count()),
        }
    }
    merge_adjacent(out)
}

/// ターミナル向けカラーマップ（ANSI）。`color` が false ならバーのみ。
pub fn format_colormap(sections: &[ContextSection], color: bool) -> String {
    if sections.is_empty() {
        return "(empty prompt)\n".to_string();
    }

    let total_chars: usize = sections.iter().map(|s| s.chars).sum();
    let total_tokens: u32 = estimated_tokens_from_chars(total_chars.max(1));
    let label_width = sections
        .iter()
        .map(|s| s.kind.label().len())
        .max()
        .unwrap_or(8);
    let bar_width = 24;

    let mut lines = vec![format!(
        "prompt sections: {} chars / ~{} tok",
        total_chars, total_tokens
    )];

    for sec in sections {
        if sec.chars == 0 {
            continue;
        }
        let pct = (sec.chars as f64) * 100.0 / (total_chars as f64);
        let tok = sec.estimated_tokens();
        let filled = ((sec.chars as f64) / (total_chars as f64) * bar_width as f64).round() as usize;
        let filled = filled.clamp(1, bar_width);
        let bar = render_bar(filled, bar_width, color, sec.kind);
        lines.push(format!(
            "{:width$} {} {:>5} tok {:>5.1}%",
            sec.kind.label(),
            bar,
            tok,
            pct,
            width = label_width
        ));
    }

    lines.join("\n")
}

fn render_bar(filled: usize, width: usize, color: bool, kind: ContextSectionKind) -> String {
    let empty = width.saturating_sub(filled);
    let blocks: String = "█".repeat(filled) + &"░".repeat(empty);
    if !color {
        return blocks;
    }
    let code = section_color(kind);
    format!("\x1b[{code}m{blocks}\x1b[0m")
}

fn section_color(kind: ContextSectionKind) -> u8 {
    match kind {
        ContextSectionKind::SystemCore => 36,    // cyan
        ContextSectionKind::ToolCatalog => 34,   // blue
        ContextSectionKind::Rules => 35,         // magenta
        ContextSectionKind::Recalled => 32,      // green
        ContextSectionKind::SystemExtra => 90,   // bright black
        ContextSectionKind::PreviousTurns => 33, // yellow
        ContextSectionKind::UserInput => 92,     // bright green
        ContextSectionKind::TurnTrace => 31,     // red
        ContextSectionKind::NextStepCue => 37,   // white
        ContextSectionKind::Other => 90,
    }
}

fn analyze_system_content(s: &str) -> Vec<ContextSection> {
    let mut out = Vec::new();

    let markers: &[(&str, ContextSectionKind)] = &[
        ("Tool catalog:\n", ContextSectionKind::ToolCatalog),
        ("\n\nAdditional rules:\n", ContextSectionKind::Rules),
        ("\n\nRecalled context:\n", ContextSectionKind::Recalled),
    ];

    let mut events: Vec<(usize, ContextSectionKind)> = markers
        .iter()
        .filter_map(|(m, k)| s.find(m).map(|i| (i, *k)))
        .collect();
    events.sort_by_key(|(i, _)| *i);

    if events.is_empty() {
        push_section(&mut out, ContextSectionKind::SystemCore, s.chars().count());
        return out;
    }

    let first = events[0].0;
    if first > 0 {
        push_section(
            &mut out,
            ContextSectionKind::SystemCore,
            s[..first].chars().count(),
        );
    }

    for (idx, (start, kind)) in events.iter().enumerate() {
        let content_start = start + marker_len(s, *kind);
        let content_end = events
            .get(idx + 1)
            .map(|(next, _)| *next)
            .unwrap_or(s.len());
        let slice = &s[content_start..content_end];
        push_section(&mut out, *kind, slice.chars().count());
    }

    out
}

fn marker_len(_s: &str, kind: ContextSectionKind) -> usize {
    match kind {
        ContextSectionKind::ToolCatalog => "Tool catalog:\n".len(),
        ContextSectionKind::Rules => "\n\nAdditional rules:\n".len(),
        ContextSectionKind::Recalled => "\n\nRecalled context:\n".len(),
        _ => 0,
    }
}

fn analyze_user_content(s: &str) -> Vec<ContextSection> {
    let mut out = Vec::new();

    let user_input = s.find("User input:\n");
    let trace = s.find("\n\nTurn trace so far:\n");
    let next = s.find("\n\nNext step JSON:");

    let mut head_end = s.len();
    if let Some(i) = user_input {
        head_end = i;
    } else if let Some(i) = trace {
        head_end = i;
    }

    if head_end > 0 {
        let head = &s[..head_end];
        if head.contains("Previous turns:") {
            push_section(&mut out, ContextSectionKind::PreviousTurns, head.chars().count());
        } else if !head.trim().is_empty() {
            push_section(&mut out, ContextSectionKind::Other, head.chars().count());
        }
    }

    if let (Some(ui), Some(tr)) = (user_input, trace) {
        let start = ui + "User input:\n".len();
        push_section(&mut out, ContextSectionKind::UserInput, s[start..tr].chars().count());
    } else if let Some(ui) = user_input {
        let start = ui + "User input:\n".len();
        let end = next.unwrap_or(s.len());
        push_section(&mut out, ContextSectionKind::UserInput, s[start..end].chars().count());
    }

    if let (Some(tr), Some(nx)) = (trace, next) {
        let start = tr + "\n\nTurn trace so far:\n".len();
        push_section(&mut out, ContextSectionKind::TurnTrace, s[start..nx].chars().count());
    } else if let Some(tr) = trace {
        let start = tr + "\n\nTurn trace so far:\n".len();
        push_section(&mut out, ContextSectionKind::TurnTrace, s[start..].chars().count());
    }

    if let Some(nx) = next {
        let start = nx + "\n\nNext step JSON:".len();
        push_section(&mut out, ContextSectionKind::NextStepCue, s[start..].chars().count());
    }

    if out.is_empty() {
        push_section(&mut out, ContextSectionKind::Other, s.chars().count());
    }

    out
}

fn push_section(out: &mut Vec<ContextSection>, kind: ContextSectionKind, chars: usize) {
    if chars == 0 {
        return;
    }
    out.push(ContextSection { kind, chars });
}

fn merge_adjacent(sections: Vec<ContextSection>) -> Vec<ContextSection> {
    let mut merged: Vec<ContextSection> = Vec::new();
    for sec in sections {
        if let Some(last) = merged.last_mut() {
            if last.kind == sec.kind {
                last.chars += sec.chars;
                continue;
            }
        }
        merged.push(sec);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_user_blocks() {
        let body = "user: Previous turns:\n[turn 1]\nUser: a\n\nUser input:\nhello\n\nTurn trace so far:\n[t0]\n\nNext step JSON:\n{}";
        let secs = analyze_prompt_body(body);
        let kinds: Vec<_> = secs.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&ContextSectionKind::PreviousTurns));
        assert!(kinds.contains(&ContextSectionKind::UserInput));
        assert!(kinds.contains(&ContextSectionKind::TurnTrace));
        assert!(kinds.contains(&ContextSectionKind::NextStepCue));
    }

    #[test]
    fn colormap_non_empty() {
        let secs = vec![
            ContextSection {
                kind: ContextSectionKind::TurnTrace,
                chars: 800,
            },
            ContextSection {
                kind: ContextSectionKind::SystemCore,
                chars: 200,
            },
        ];
        let map = format_colormap(&secs, false);
        assert!(map.contains("turn_trace"));
        assert!(map.contains("█"));
    }
}
