use std::collections::HashSet;

use crate::plan::control_plane_catalog_footer;

use super::{task_available_with_tools, TaskRegistry};

impl TaskRegistry {
    /// 計画 LLM 向けカタログ（必須実行順序付き）。
    pub fn catalog_for_planner(&self) -> String {
        self.catalog_for_planner_opts(true)
    }

    /// 計画層向けタスク一覧。`include_web_research` が false のとき `web_research` を除外する。
    pub fn catalog_for_planner_opts(&self, include_web_research: bool) -> String {
        self.catalog_for_planner_filtered(&HashSet::new(), include_web_research, &[], false)
    }

    /// 実行層に登録済みのツール名に基づき、計画可能な task id だけを載せる。
    /// `available_tools` が空かつ `require_all_tools` が false のときは全タスク（従来どおり）。
    pub fn catalog_for_planner_filtered(
        &self,
        available_tools: &HashSet<String>,
        include_web_research: bool,
        exclude_task_ids: &[&str],
        require_all_tools: bool,
    ) -> String {
        let filter_by_tools = require_all_tools || !available_tools.is_empty();
        let mut lines =
            vec!["Registered tasks for this session (use only task ids listed here):".into()];
        if filter_by_tools {
            let mut names: Vec<_> = available_tools.iter().map(String::as_str).collect();
            names.sort();
            lines.push(format!(
                "Available execution tools: {}",
                if names.is_empty() {
                    "(none)".into()
                } else {
                    names.join(", ")
                }
            ));
        }
        let mut ids: Vec<_> = self.tasks.keys().collect();
        ids.sort();
        for id in ids {
            if exclude_task_ids.contains(&id.as_str()) {
                continue;
            }
            if *id == "web_research" && !include_web_research {
                continue;
            }
            let def = &self.tasks[id];
            if filter_by_tools && !task_available_with_tools(def, available_tools) {
                continue;
            }
            let steps = def
                .ordered_required_steps()
                .iter()
                .map(|s| s.method.as_str())
                .collect::<Vec<_>>()
                .join(" → ");
            let steps_part = if steps.is_empty() {
                "(free execution — pick tools from catalog above)".into()
            } else {
                format!("required: {steps}")
            };
            lines.push(format!("- {id}: {} — {steps_part}", def.summary));
        }
        if lines.len() <= 1 {
            lines.push("- generic: (free execution)".into());
        }
        let mut out = lines.join("\n");
        out.push_str(control_plane_catalog_footer());
        out
    }

    /// 計画候補選定用: id + planner_summary のみ（手順の詳細は載せない）。
    pub fn catalog_summaries_for_planner(
        &self,
        available_tools: &HashSet<String>,
        include_web_research: bool,
        exclude_task_ids: &[&str],
        require_all_tools: bool,
    ) -> String {
        self.catalog_summaries_for_planner_budgeted(
            available_tools,
            include_web_research,
            exclude_task_ids,
            require_all_tools,
            usize::MAX,
            usize::MAX,
        )
    }

    /// 件数・文字数キャップ付き summary カタログ。
    pub fn catalog_summaries_for_planner_budgeted(
        &self,
        available_tools: &HashSet<String>,
        include_web_research: bool,
        exclude_task_ids: &[&str],
        require_all_tools: bool,
        max_entries: usize,
        max_chars: usize,
    ) -> String {
        let filter_by_tools = require_all_tools || !available_tools.is_empty();
        let mut lines =
            vec!["Task candidates (summaries only — pick ids that fit the user goal):".into()];
        let mut ids: Vec<_> = self.tasks.keys().collect();
        ids.sort();
        let mut entry_count = 0usize;
        for id in ids {
            if exclude_task_ids.contains(&id.as_str()) {
                continue;
            }
            if *id == "web_research" && !include_web_research {
                continue;
            }
            let def = &self.tasks[id];
            if filter_by_tools && !task_available_with_tools(def, available_tools) {
                continue;
            }
            if entry_count >= max_entries {
                lines.push(format!("- … ({entry_count}+ more omitted; catalog budget)"));
                break;
            }
            let line = format!("- {id}: {}", def.effective_planner_summary());
            let projected = lines.iter().map(|l| l.len() + 1).sum::<usize>() + line.len();
            if projected > max_chars && entry_count > 0 {
                lines.push(format!(
                    "- … (truncated at {max_chars} chars; catalog budget)"
                ));
                break;
            }
            lines.push(line);
            entry_count += 1;
        }
        if lines.len() <= 1 {
            lines.push("- generic: Freeform ReAct when no specialized task fits.".into());
        }
        let mut out = lines.join("\n");
        out.push_str(control_plane_catalog_footer());
        out
    }

    /// 選ばれた候補 id だけの詳細カタログ（手順・required tools）。
    pub fn catalog_for_candidate_ids(
        &self,
        candidate_ids: &[String],
        available_tools: &HashSet<String>,
        include_web_research: bool,
        require_all_tools: bool,
    ) -> String {
        let mut allow: HashSet<&str> = candidate_ids.iter().map(String::as_str).collect();
        if allow.is_empty() {
            allow.insert("generic");
        }
        // generic は常にフォールバックとして残せる
        if !allow.contains("generic") && self.tasks.contains_key("generic") {
            allow.insert("generic");
        }
        let exclude: Vec<&str> = self
            .tasks
            .keys()
            .filter(|id| !allow.contains(id.as_str()))
            .map(|s| s.as_str())
            .collect();
        let mut catalog = self.catalog_for_planner_filtered(
            available_tools,
            include_web_research,
            &exclude,
            require_all_tools,
        );
        catalog.push_str(
            "\n\nOnly use registered task ids listed above (selected for this turn), \
plus control-plane ids from the footer when needed. Prefer them over inventing freeform steps.",
        );
        catalog
    }

    /// 候補タスクが必要とするツール名（steps + tool_policy.allow）。`generic` 含む場合は None（全ツール）。
    pub fn tools_for_candidate_ids(&self, candidate_ids: &[String]) -> Option<HashSet<String>> {
        if candidate_ids.iter().any(|id| id == "generic") {
            return None;
        }
        let mut tools = HashSet::new();
        for id in candidate_ids {
            let Some(def) = self.get(id) else {
                continue;
            };
            if def.steps.is_empty() && def.tool_policy.allow.is_empty() {
                // 契約なし ≈ 自由 → 全ツール
                return None;
            }
            for step in &def.steps {
                tools.insert(step.method.clone());
            }
            for name in &def.tool_policy.allow {
                tools.insert(name.clone());
            }
        }
        if tools.is_empty() {
            None
        } else {
            Some(tools)
        }
    }

    /// 登録タスク id のうち、利用可能ツールで実行可能なもの。
    pub fn available_task_ids(
        &self,
        available_tools: &HashSet<String>,
        include_web_research: bool,
        exclude_task_ids: &[&str],
        require_all_tools: bool,
    ) -> Vec<String> {
        let filter_by_tools = require_all_tools || !available_tools.is_empty();
        let mut ids: Vec<_> = self.tasks.keys().cloned().collect();
        ids.sort();
        ids.into_iter()
            .filter(|id| {
                if exclude_task_ids.contains(&id.as_str()) {
                    return false;
                }
                if id == "web_research" && !include_web_research {
                    return false;
                }
                let def = &self.tasks[id];
                !filter_by_tools || task_available_with_tools(def, available_tools)
            })
            .collect()
    }
}
