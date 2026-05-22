use std::fmt::Write;

/// 完了した 1 REPL ターン（ユーザー入力と最終回答）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastTurn {
    pub user_input: String,
    pub answer: String,
}

/// REPL セッション内の短期記憶（直近 N ターンをプロンプトへ注入）。
#[derive(Debug, Clone)]
pub struct SessionMemory {
    turns: Vec<PastTurn>,
    max_turns: usize,
    max_chars_per_field: usize,
}

impl SessionMemory {
    pub const DEFAULT_MAX_TURNS: usize = 8;
    pub const DEFAULT_MAX_CHARS_PER_FIELD: usize = 2000;

    pub fn new(max_turns: usize) -> Self {
        Self {
            turns: Vec::new(),
            max_turns: max_turns.max(1),
            max_chars_per_field: Self::DEFAULT_MAX_CHARS_PER_FIELD,
        }
    }

    pub fn with_limits(max_turns: usize, max_chars_per_field: usize) -> Self {
        let mut s = Self::new(max_turns);
        s.max_chars_per_field = max_chars_per_field.max(1);
        s
    }

    pub fn len(&self) -> usize {
        self.turns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    pub fn turns(&self) -> &[PastTurn] {
        &self.turns
    }

    pub fn clear(&mut self) {
        self.turns.clear();
    }

    /// ターン終了時に呼ぶ（古いターンは先頭から捨てる）。
    pub fn push_turn(&mut self, user_input: impl Into<String>, answer: impl Into<String>) {
        self.turns.push(PastTurn {
            user_input: truncate_field(user_input.into(), self.max_chars_per_field),
            answer: truncate_field(answer.into(), self.max_chars_per_field),
        });
        while self.turns.len() > self.max_turns {
            self.turns.remove(0);
        }
    }

    /// `Previous turns:` セクション（空なら空文字）。
    pub fn format_for_prompt(&self) -> String {
        if self.turns.is_empty() {
            return String::new();
        }
        let mut out = String::from("Previous turns:\n");
        for (i, t) in self.turns.iter().enumerate() {
            let n = i + 1;
            writeln!(out, "[turn {n}]").ok();
            writeln!(out, "User: {}", t.user_input).ok();
            writeln!(out, "Assistant: {}", t.answer).ok();
        }
        out
    }
}

impl Default for SessionMemory {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_TURNS)
    }
}

fn truncate_field(s: String, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s;
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_session_formats_empty() {
        let s = SessionMemory::new(4);
        assert!(s.format_for_prompt().is_empty());
    }

    #[test]
    fn format_includes_user_and_answer() {
        let mut s = SessionMemory::new(4);
        s.push_turn("first question", "first answer");
        let text = s.format_for_prompt();
        assert!(text.contains("Previous turns:"));
        assert!(text.contains("[turn 1]"));
        assert!(text.contains("User: first question"));
        assert!(text.contains("Assistant: first answer"));
    }

    #[test]
    fn drops_oldest_when_over_max() {
        let mut s = SessionMemory::new(2);
        s.push_turn("a", "1");
        s.push_turn("b", "2");
        s.push_turn("c", "3");
        assert_eq!(s.len(), 2);
        assert_eq!(s.turns()[0].user_input, "b");
        assert_eq!(s.turns()[1].user_input, "c");
    }

    #[test]
    fn truncates_long_answer() {
        let mut s = SessionMemory::with_limits(4, 10);
        s.push_turn("x", "abcdefghijklmnop");
        assert!(s.turns()[0].answer.chars().count() <= 11);
        assert!(s.turns()[0].answer.len() < "abcdefghijklmnop".len());
    }
}
