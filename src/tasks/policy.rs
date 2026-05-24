//! サブタスク単位のツール allow / deny（実行層の遮蔽・監査用）。

use std::collections::HashSet;

use serde::Deserialize;

use super::spec::TaskDefinition;

/// `tasks/*.json` の `tool_policy` 節。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct ToolPolicySpec {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// 実行中サブタスクに適用する解決済みポリシー。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubtaskToolPolicy {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

impl SubtaskToolPolicy {
    pub fn is_allowed(&self, tool: &str) -> bool {
        if self.deny.iter().any(|d| d == tool) {
            return false;
        }
        if self.allow.is_empty() {
            return true;
        }
        self.allow.iter().any(|a| a == tool)
    }

    pub fn format_for_mission(&self) -> String {
        let mut out = String::from("Tool policy for this subtask only:\n");
        if self.allow.is_empty() {
            out.push_str("- Allowed: any registered tool (except denied below)\n");
        } else {
            out.push_str("- Allowed tools: ");
            out.push_str(&self.allow.join(", "));
            out.push('\n');
        }
        if !self.deny.is_empty() {
            out.push_str("- Denied tools (do not call): ");
            out.push_str(&self.deny.join(", "));
            out.push('\n');
        }
        out.push_str("- Tools not listed in the Tool catalog below are not available.\n");
        out
    }
}

impl TaskDefinition {
    pub fn resolved_tool_policy(&self) -> SubtaskToolPolicy {
        let mut allow: Vec<String> = self
            .tool_policy
            .allow
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        for step in &self.steps {
            let m = step.method.trim();
            if !m.is_empty() && !allow.iter().any(|a| a == m) {
                allow.push(m.to_string());
            }
        }

        if allow.is_empty() && !self.steps.is_empty() {
            allow = self
                .steps
                .iter()
                .map(|s| s.method.clone())
                .collect();
        }

        let mut seen = HashSet::new();
        allow.retain(|s| seen.insert(s.clone()));

        let deny: Vec<String> = self
            .tool_policy
            .deny
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        SubtaskToolPolicy { allow, deny }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn compose_context() -> TaskDefinition {
        serde_json::from_value(json!({
            "id": "compose_context",
            "summary": "context",
            "steps": [{"order": 1, "method": "get_compose_form", "args": {}}],
            "tool_policy": {
                "allow": ["get_compose_form", "get_email"],
                "deny": ["set_compose_form"]
            }
        }))
        .unwrap()
    }

    #[test]
    fn resolves_allow_and_deny() {
        let def = compose_context();
        let p = def.resolved_tool_policy();
        assert!(p.is_allowed("get_compose_form"));
        assert!(p.is_allowed("get_email"));
        assert!(!p.is_allowed("set_compose_form"));
        assert!(!p.is_allowed("fetch_mails"));
    }

    #[test]
    fn deny_wins_over_allow() {
        let def: TaskDefinition = serde_json::from_value(json!({
            "id": "t",
            "summary": "t",
            "tool_policy": {
                "allow": ["set_compose_form"],
                "deny": ["set_compose_form"]
            }
        }))
        .unwrap();
        assert!(!def.resolved_tool_policy().is_allowed("set_compose_form"));
    }
}
