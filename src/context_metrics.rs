use std::fmt;

use crate::llm::ChatMessage;

/// テキストの文字数・バイト数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextSize {
    pub chars: usize,
    pub bytes: usize,
}

impl TextSize {
    pub fn measure(text: &str) -> Self {
        Self {
            chars: text.chars().count(),
            bytes: text.len(),
        }
    }

    pub fn estimated_tokens(&self) -> u32 {
        estimated_tokens_from_chars(self.chars)
    }
}

/// トークン数の出所。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TokenSource {
    /// API の `usage`（OpenAI 互換 / Ollama）。
    Api,
    /// 文字数からの概算（約 4 文字 / トークン）。
    #[default]
    Estimated,
}

/// 1 回の LLM 呼び出しのコンテキスト計測。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextUsage {
    /// API に送ったプロンプト全文（`role: content` 形式）。
    pub prompt_body: String,
    pub prompt: TextSize,
    pub completion: TextSize,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub token_source: TokenSource,
}

impl ContextUsage {
    pub fn prompt_tokens_effective(&self) -> u32 {
        self.prompt_tokens
            .unwrap_or_else(|| self.prompt.estimated_tokens())
    }

    pub fn completion_tokens_effective(&self) -> u32 {
        self.completion_tokens
            .unwrap_or_else(|| self.completion.estimated_tokens())
    }

    pub fn total_tokens_effective(&self) -> u32 {
        self.prompt_tokens_effective() + self.completion_tokens_effective()
    }

    pub fn from_parts(
        prompt_body: &str,
        completion_text: &str,
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
    ) -> Self {
        let token_source = if prompt_tokens.is_some() || completion_tokens.is_some() {
            TokenSource::Api
        } else {
            TokenSource::Estimated
        };
        Self {
            prompt_body: prompt_body.to_string(),
            prompt: TextSize::measure(prompt_body),
            completion: TextSize::measure(completion_text),
            prompt_tokens,
            completion_tokens,
            token_source,
        }
    }

    pub fn measure_messages(messages: &[ChatMessage], completion_text: &str) -> Self {
        Self::from_parts(
            &format_messages_body(messages),
            completion_text,
            None,
            None,
        )
    }
}

/// Chat Completions に送るメッセージ列をログ用テキストにする。
pub fn format_messages_body(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|m| format!("{}: {}\n", m.role, m.content))
        .collect()
}

/// 1 ターン内の LLM 呼び出しを合算。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnContextSummary {
    pub llm_calls: usize,
    pub prompt: TextSize,
    pub completion: TextSize,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub token_source: TokenSource,
}

impl TurnContextSummary {
    pub fn from_usages(usages: &[ContextUsage]) -> Self {
        if usages.is_empty() {
            return Self::default();
        }

        let mut prompt = TextSize::default();
        let mut completion = TextSize::default();
        let mut prompt_tokens = 0u32;
        let mut completion_tokens = 0u32;
        let mut all_api = true;

        for u in usages {
            prompt.chars += u.prompt.chars;
            prompt.bytes += u.prompt.bytes;
            completion.chars += u.completion.chars;
            completion.bytes += u.completion.bytes;
            prompt_tokens += u.prompt_tokens_effective();
            completion_tokens += u.completion_tokens_effective();
            if u.token_source != TokenSource::Api {
                all_api = false;
            }
        }

        Self {
            llm_calls: usages.len(),
            prompt,
            completion,
            prompt_tokens,
            completion_tokens,
            token_source: if all_api {
                TokenSource::Api
            } else {
                TokenSource::Estimated
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.llm_calls == 0
    }
}

impl fmt::Display for ContextUsage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let src = match self.token_source {
            TokenSource::Api => "api",
            TokenSource::Estimated => "est",
        };
        write!(
            f,
            "prompt: {} chars / {} tok | completion: {} chars / {} tok ({src})",
            self.prompt.chars,
            self.prompt_tokens_effective(),
            self.completion.chars,
            self.completion_tokens_effective(),
        )
    }
}

impl fmt::Display for TurnContextSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return write!(f, "(no LLM calls)");
        }
        let src = match self.token_source {
            TokenSource::Api => "api",
            TokenSource::Estimated => "est",
        };
        write!(
            f,
            "llm_calls={} prompt: {} chars / {} tok ({src}) | completion: {} chars / {} tok ({src}) | total_tokens={}",
            self.llm_calls,
            self.prompt.chars,
            self.prompt_tokens,
            self.completion.chars,
            self.completion_tokens,
            self.prompt_tokens + self.completion_tokens,
        )
    }
}

pub fn estimated_tokens_from_chars(chars: usize) -> u32 {
    chars.div_ceil(4) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_size_counts_utf8() {
        let s = TextSize::measure("日本語");
        assert_eq!(s.chars, 3);
        assert!(s.bytes > 3);
    }

    #[test]
    fn turn_summary_sums_usages() {
        let u1 = ContextUsage::from_parts("aaaa", "bb", Some(10), Some(5));
        let u2 = ContextUsage::from_parts("cc", "dd", None, None);
        let sum = TurnContextSummary::from_usages(&[u1, u2]);
        assert_eq!(sum.llm_calls, 2);
        assert_eq!(sum.prompt.chars, 6);
        assert_eq!(sum.prompt_tokens, 10 + 1);
    }
}
