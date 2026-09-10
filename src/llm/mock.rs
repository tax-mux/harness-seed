use super::completion::CompletionResult;
use super::connector::{ChatMessage, ConnectorError, LlmConnector};
use crate::context_metrics::ContextUsage;

/// テスト用の決定的コネクタ（HTTP なし）。
#[derive(Debug, Default)]
pub struct MockLlmConnector;

impl LlmConnector for MockLlmConnector {
    fn complete(&self, messages: &[ChatMessage]) -> Result<CompletionResult, ConnectorError> {
        let last_user = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let plan_answer = r#"{"step":"answer","content":"{\"summary\":\"mock plan\",\"skip_execution\":false,\"subtasks\":[{\"id\":1,\"goal\":\"first step\",\"done_when\":\"done\"},{\"id\":2,\"goal\":\"second step\",\"done_when\":\"done\"}]}"}"#;
        let plan_list_dir = r#"{"step":"answer","content":"{\"summary\":\"list\",\"skip_execution\":false,\"subtasks\":[{\"id\":1,\"task\":\"list_dir\",\"params\":{\"path\":\"src\"},\"goal\":\"\",\"done_when\":\"\"}]}"}"#;

        let content = if last_user.contains("Next plan step JSON") {
            if last_user.contains("STEP_DRIVER_TEST") {
                if last_user.contains("[thought") {
                    plan_list_dir
                } else {
                    r#"{"step":"thought","content":"plan for driver"}"#
                }
            } else if last_user.contains("[thought") {
                plan_answer
            } else {
                r#"{"step":"thought","content":"mock plan thought"}"#
            }
        } else if last_user.contains("Current subtask:") {
            let id = if last_user.contains("Current subtask: 2") {
                "2"
            } else {
                "1"
            };
            &format!(r#"{{"step":"answer","content":"subtask \"{id}\" done"}}"#)
        } else if last_user.contains("[observation") {
            r#"{"step":"answer","content":"mock answer"}"#
        } else if last_user.contains("[thought") {
            r#"{"step":"action","tool":"echo","args":{"message":"from-mock"}}"#
        } else {
            r#"{"step":"thought","content":"mock thought"}"#
        };

        let usage = ContextUsage::measure_messages(messages, content);
        Ok(CompletionResult {
            content: content.into(),
            usage,
        })
    }
}
