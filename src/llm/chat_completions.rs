use reqwest::blocking::Client;
use reqwest::blocking::RequestBuilder;
use serde::Deserialize;

use super::completion::CompletionResult;
use super::connector::{ChatMessage, ConnectorError, LlmConfig, LlmConnector};
use crate::context_metrics::{format_messages_body, ContextUsage};

/// OpenAI 互換 Chat Completions API クライアント（OpenAI / Ollama / LM Studio 共通）。
#[derive(Debug)]
pub struct ChatCompletionsConnector {
    client: Client,
    config: LlmConfig,
}

impl ChatCompletionsConnector {
    pub fn new(config: LlmConfig) -> Result<Self, ConnectorError> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()?;
        Ok(Self { client, config })
    }

    pub fn config(&self) -> &LlmConfig {
        &self.config
    }

    fn apply_auth(&self, request: RequestBuilder) -> RequestBuilder {
        if let Some(key) = &self.config.api_key {
            if !key.is_empty() {
                return request.bearer_auth(key);
            }
        }
        request
    }
}

#[derive(serde::Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(serde::Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: &'static str,
}

#[derive(Deserialize, Default)]
struct Usage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Usage,
    prompt_eval_count: Option<u32>,
    eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

fn resolve_token_usage(parsed: &ChatResponse) -> (Option<u32>, Option<u32>) {
    let mut prompt = parsed.usage.prompt_tokens;
    let mut completion = parsed.usage.completion_tokens;

    if prompt.is_none() {
        prompt = parsed.prompt_eval_count;
    }
    if completion.is_none() {
        completion = parsed.eval_count;
    }

    (prompt, completion)
}

impl LlmConnector for ChatCompletionsConnector {
    fn provider(&self) -> super::connector::LlmProvider {
        self.config.provider
    }

    fn complete(&self, messages: &[ChatMessage]) -> Result<CompletionResult, ConnectorError> {
        let url = format!("{}/chat/completions", self.config.base_url);
        let response_format = self.config.json_mode.then(|| ResponseFormat {
            format_type: "json_object",
        });

        let body = ChatRequest {
            model: &self.config.model,
            messages,
            temperature: 0.2,
            response_format,
        };

        let request = self.client.post(&url).json(&body);
        let response = self.apply_auth(request).send()?;

        let status = response.status();
        let text = response.text()?;
        if !status.is_success() {
            return Err(ConnectorError::Http {
                status: status.as_u16(),
                body: text,
            });
        }

        let parsed: ChatResponse = serde_json::from_str(&text)
            .map_err(|e| ConnectorError::InvalidResponse(format!("{e}; body={text}")))?;

        let (prompt_tokens, completion_tokens) = resolve_token_usage(&parsed);

        let content = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| ConnectorError::InvalidResponse("empty choices".into()))?;

        let usage = ContextUsage::from_parts(
            &format_messages_body(messages),
            &content,
            prompt_tokens,
            completion_tokens,
        );

        Ok(CompletionResult { content, usage })
    }
}
