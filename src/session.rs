use std::fmt::Write;

/// 完了した 1 REPL ターン（ユーザー入力と最終回答）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastTurn {
    pub user_input: String,
    pub answer: String,
}

/// Previous turns をプロンプトに載せるか（記憶 RAG の `work_log` と揃える）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionPromptPolicy {
    /// 作業ログ経路: 直近ターンを載せる。
    IncludePrior,
    /// 知識のみ / 不要: Previous turns を載せない。
    #[default]
    OmitPrior,
}

/// REPL セッション内の短期記憶（直近 N ターンをプロンプトへ注入）。
#[derive(Debug, Clone)]
pub struct SessionMemory {
    turns: Vec<PastTurn>,
    max_turns: usize,
    max_chars_per_field: usize,
    prompt_policy: SessionPromptPolicy,
}

impl SessionMemory {
    pub const DEFAULT_MAX_TURNS: usize = 8;
    pub const DEFAULT_MAX_CHARS_PER_FIELD: usize = 2000;

    pub fn new(max_turns: usize) -> Self {
        Self {
            turns: Vec::new(),
            max_turns: max_turns.max(1),
            max_chars_per_field: Self::DEFAULT_MAX_CHARS_PER_FIELD,
            prompt_policy: SessionPromptPolicy::OmitPrior,
        }
    }

    /// 記憶 RAG の `work_log` に合わせて Previous turns の出し分けを設定する。
    pub fn set_prompt_policy(&mut self, policy: SessionPromptPolicy) {
        self.prompt_policy = policy;
    }

    pub fn prompt_policy(&self) -> SessionPromptPolicy {
        self.prompt_policy
    }

    /// ルータ用の直前ターン一行要約（全文は渡さない）。
    pub fn prior_one_liner(&self) -> Option<String> {
        self.turns.last().map(|t| {
            let answer = truncate_field(t.answer.clone(), 120);
            format!("User: {} | Assistant: {answer}", t.user_input)
        })
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
    ///
    /// [`SessionPromptPolicy::IncludePrior`]（作業ログ経路）のときだけ載せる。
    pub fn format_for_prompt(&self) -> String {
        if self.prompt_policy != SessionPromptPolicy::IncludePrior || self.turns.is_empty() {
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
    fn format_includes_user_and_answer_when_work_log() {
        let mut s = SessionMemory::new(4);
        s.push_turn("first question", "first answer");
        s.set_prompt_policy(SessionPromptPolicy::IncludePrior);
        let text = s.format_for_prompt();
        assert!(text.contains("Previous turns:"));
        assert!(text.contains("[turn 1]"));
        assert!(text.contains("User: first question"));
        assert!(text.contains("Assistant: first answer"));
    }

    #[test]
    fn format_omits_prior_on_knowledge_route() {
        let mut s = SessionMemory::new(4);
        s.push_turn(
            "このプロジェクトについて説明して",
            "HarnessSeed は ReAct harness です",
        );
        s.set_prompt_policy(SessionPromptPolicy::OmitPrior);
        assert!(s.format_for_prompt().is_empty());
        s.set_prompt_policy(SessionPromptPolicy::IncludePrior);
        assert!(s.format_for_prompt().contains("HarnessSeed"));
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
