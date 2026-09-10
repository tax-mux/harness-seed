use std::fmt;

use serde::Deserialize;
use serde_json::Value;

/// タスク内の 1 実行ステップ（必須メソッド + 順序）。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ExecStep {
    /// 実行順（1 始まり。小さいほど先）。
    pub order: u32,
    /// 実行メソッド名（組み込みツール名。例: `list_dir`, `write_file`）。
    pub method: String,
    /// 引数テンプレート（`{param}` プレースホルダ可）。
    #[serde(default)]
    pub args: Value,
    /// false のとき監査ではスキップ可（スケルトン: 省略可ステップ用）。
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

/// ファイル（`tasks/*.json`）から読み込むタスク定義。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TaskDefinition {
    pub id: String,
    pub summary: String,
    #[serde(default)]
    pub default_params: Value,
    /// 必須実行メソッドと順序。空なら自由実行（`generic`）。
    #[serde(default)]
    pub steps: Vec<ExecStep>,
    /// 全ステップ完了後の完了条件（テキスト）。
    #[serde(default)]
    pub done_when: String,
}

impl TaskDefinition {
    /// `order` 昇順の必須ステップ列。
    pub fn ordered_required_steps(&self) -> Vec<&ExecStep> {
        let mut steps: Vec<_> = self.steps.iter().filter(|s| s.required).collect();
        steps.sort_by_key(|s| s.order);
        steps
    }

    /// `order` 昇順の全ステップ列（ステップドライバ用）。
    pub fn ordered_steps(&self) -> Vec<&ExecStep> {
        let mut steps: Vec<_> = self.steps.iter().collect();
        steps.sort_by_key(|s| s.order);
        steps
    }

    /// 定義の整合性チェック。
    pub fn validate_definition(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for step in &self.steps {
            if step.method.trim().is_empty() {
                return Err(format!("task '{}': step order {} has empty method", self.id, step.order));
            }
            if !seen.insert(step.order) {
                return Err(format!(
                    "task '{}': duplicate step order {}",
                    self.id, step.order
                ));
            }
        }
        Ok(())
    }

    /// 展開済み params で必須実行順序を人間／LLM 向けに列挙する。
    pub fn format_required_execution(&self, params: &Value) -> String {
        let required = self.ordered_required_steps();
        if required.is_empty() {
            return "Required execution: (none — ReAct may choose tools freely)\n".into();
        }
        let mut out = String::from(
            "Required execution order (complete methods in this order; do not skip):\n",
        );
        for step in required {
            let args = apply_template_value(&step.args, params);
            out.push_str(&format!(
                "  {}. {}({})\n",
                step.order,
                step.method,
                args
            ));
        }
        if !self.done_when.is_empty() {
            let dw = apply_template(&self.done_when, params);
            out.push_str(&format!("\nDone when all above: {dw}\n"));
        }
        out
    }

    /// 固定契約があるか（`generic` など steps 空は false）。
    pub fn has_execution_contract(&self) -> bool {
        !self.ordered_required_steps().is_empty()
    }
}

/// mission 組み立て用の文脈。
#[derive(Debug, Clone)]
pub struct MissionRenderContext<'a> {
    pub original_request: &'a str,
    pub plan_summary: &'a str,
    pub subtask_list: &'a str,
    pub prior_results: &'a str,
    pub subtask_id: u32,
    pub goal: &'a str,
    pub done_when: &'a str,
    pub params: &'a Value,
}

#[derive(Debug)]
pub enum TaskError {
    UnknownTask { id: String },
    InvalidDefinition { id: String, reason: String },
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTask { id } => write!(f, "unknown task id: {id}"),
            Self::InvalidDefinition { id, reason } => {
                write!(f, "invalid task definition '{id}': {reason}")
            }
        }
    }
}

impl std::error::Error for TaskError {}

/// `{key}` プレースホルダを params で置換（文字列テンプレート用）。
pub fn apply_template(template: &str, params: &Value) -> String {
    let Some(map) = params.as_object() else {
        return template.to_string();
    };
    let mut out = template.to_string();
    for (key, value) in map {
        let placeholder = format!("{{{key}}}");
        let replacement = value_to_string(value);
        out = out.replace(&placeholder, &replacement);
    }
    out
}

/// JSON 値ツリー内の文字列葉を再帰的に展開。
pub fn apply_template_value(template: &Value, params: &Value) -> Value {
    match template {
        Value::String(s) => Value::String(apply_template(s, params)),
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|v| apply_template_value(v, params))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), apply_template_value(v, params)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn apply_template_replaces_placeholders() {
        let t = "path={path} done={done_when}";
        let p = json!({ "path": "src", "done_when": "ok" });
        assert_eq!(apply_template(t, &p), "path=src done=ok");
    }

    #[test]
    fn ordered_required_steps_sorts_by_order() {
        let def: TaskDefinition = serde_json::from_str(
            r#"{
            "id": "t",
            "summary": "s",
            "steps": [
                {"order": 2, "method": "read_file", "args": {}},
                {"order": 1, "method": "write_file", "args": {}}
            ]
        }"#,
        )
        .unwrap();
        let methods: Vec<_> = def
            .ordered_required_steps()
            .iter()
            .map(|s| s.method.as_str())
            .collect();
        assert_eq!(methods, vec!["write_file", "read_file"]);
    }
}
