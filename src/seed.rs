//! ホスト資産登録の正本（`SeedBuilder`）。
//!
//! 埋め込みホストはここだけを覚えればよい。CLI / 単体稼働は資産を集めて
//! [`SeedBuilder::merge_agent_project`] 等へ流すヘルパーに留める。

use std::path::Path;
use std::sync::Arc;

use crate::agent_assets::{
    apply_workspace_env, load_agent_assets, AgentLoadError, AgentLoadReport, AgentProjectConfig,
};
use crate::brain::AgentBrain;
use crate::brave_search::BraveSearchConfig;
use crate::config::AppConfig;
use crate::context::{ContextError, PromptBlocks};
use crate::lifecycle::TurnLifecycle;
use crate::memory::{MemoryBridge, MemoryRag, NoopBridge};
use crate::plan::{PlanBrainMode, PlanDataContract};
use crate::react::{ReActConfig, ReActLoop};
use crate::tasks::TaskRegistry;
use crate::tool::{default_packs, Tool, ToolPack};

/// セッション起動時のホスト配線を組み立てるビルダ（ライブラリ正本）。
pub struct SeedBuilder {
    blocks: PromptBlocks,
    task_registry: TaskRegistry,
    plugins: Vec<Box<dyn Tool>>,
    lifecycle: Option<Arc<dyn TurnLifecycle>>,
    brave_search: Option<BraveSearchConfig>,
    tool_packs: Vec<ToolPack>,
    memory: Box<dyn MemoryBridge>,
    memory_rag: Option<MemoryRag>,
    agent_report: Option<AgentLoadReport>,
}

impl Default for SeedBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SeedBuilder {
    /// 空のブロックと既定タスク／パックから始める。
    pub fn new() -> Self {
        Self {
            blocks: PromptBlocks::new(),
            task_registry: TaskRegistry::load_default(),
            plugins: Vec::new(),
            lifecycle: None,
            brave_search: None,
            tool_packs: default_packs(false),
            memory: Box::new(NoopBridge),
            memory_rag: None,
            agent_report: None,
        }
    }

    /// `AppConfig` から rules / packs / brave / memory を取り込む。
    pub fn from_app(app: &AppConfig) -> Result<Self, ContextError> {
        Ok(Self {
            blocks: app.load_prompt_blocks()?,
            task_registry: TaskRegistry::load_default(),
            plugins: Vec::new(),
            lifecycle: None,
            brave_search: app.resolved_brave_search(),
            tool_packs: app.resolved_tool_packs(),
            memory: app.memory_bridge(),
            memory_rag: None,
            agent_report: None,
        })
    }

    pub fn prompt_blocks(mut self, blocks: PromptBlocks) -> Self {
        self.blocks = blocks;
        self
    }

    pub fn task_registry(mut self, task_registry: TaskRegistry) -> Self {
        self.task_registry = task_registry;
        self
    }

    pub fn plugin(mut self, tool: Box<dyn Tool>) -> Self {
        self.plugins.push(tool);
        self
    }

    pub fn plugins(mut self, tools: impl IntoIterator<Item = Box<dyn Tool>>) -> Self {
        self.plugins.extend(tools);
        self
    }

    pub fn lifecycle(mut self, lifecycle: Arc<dyn TurnLifecycle>) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    pub fn clear_lifecycle(mut self) -> Self {
        self.lifecycle = None;
        self
    }

    pub fn plan_data_contract(mut self, contract: PlanDataContract) -> Self {
        self.blocks.plan_data_contract = Some(contract);
        self
    }

    pub fn clear_plan_data_contract(mut self) -> Self {
        self.blocks.plan_data_contract = None;
        self
    }

    pub fn brave_search(mut self, brave: Option<BraveSearchConfig>) -> Self {
        self.brave_search = brave;
        self
    }

    pub fn tool_packs(mut self, packs: Vec<ToolPack>) -> Self {
        self.tool_packs = packs;
        self
    }

    pub fn memory_bridge(mut self, memory: Box<dyn MemoryBridge>) -> Self {
        self.memory = memory;
        self
    }

    pub fn memory_rag(mut self, memory_rag: MemoryRag) -> Self {
        self.memory_rag = Some(memory_rag);
        self
    }

    /// `.agent` 等のプロジェクト資産を正本へマージする（CLI / 単体ヘルパー用）。
    pub fn merge_agent_project(
        mut self,
        config: &AgentProjectConfig,
        base_dir: &Path,
    ) -> Result<Self, AgentLoadError> {
        let paths = config.resolve_paths(base_dir)?;
        apply_workspace_env(&paths.workspace);
        self.blocks.context_manifest_path = config.resolved_context_manifest_path(base_dir)?;
        let (report, tools) =
            load_agent_assets(&paths, &mut self.blocks, &mut self.task_registry)?;
        self.plugins.extend(tools);
        self.agent_report = Some(report);
        Ok(self)
    }

    pub fn agent_report(&self) -> Option<&AgentLoadReport> {
        self.agent_report.as_ref()
    }

    /// 計画頭脳組み立て用。`build` 前に最終レジストリを渡す。
    pub fn task_registry_ref(&self) -> &TaskRegistry {
        &self.task_registry
    }

    pub fn blocks_ref(&self) -> &PromptBlocks {
        &self.blocks
    }

    pub fn tool_packs_ref(&self) -> &[ToolPack] {
        &self.tool_packs
    }

    pub fn brave_search_ref(&self) -> Option<&BraveSearchConfig> {
        self.brave_search.as_ref()
    }

    /// 頭脳と ReAct 設定を渡してループを完成させる。
    pub fn build<E: AgentBrain>(
        self,
        exec_brain: E,
        plan_brain: PlanBrainMode,
        config: ReActConfig,
    ) -> ReActLoop<E> {
        let Self {
            blocks,
            task_registry,
            plugins,
            lifecycle,
            brave_search,
            tool_packs,
            memory,
            memory_rag,
            agent_report: _,
        } = self;

        let mut react = ReActLoop::with_blocks_and_tasks(
            exec_brain,
            plan_brain,
            config,
            blocks,
            task_registry,
            brave_search,
            &tool_packs,
            memory,
        );
        for tool in plugins {
            react.register_plugin(tool);
        }
        if let Some(lc) = lifecycle {
            react.set_lifecycle(Some(lc));
        }
        if let Some(rag) = memory_rag {
            react.set_memory_rag(rag);
        }
        // register_plugin が都度 refresh するが、契約・計画カタログを最終確定する。
        react.refresh_tool_catalog();
        react
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::SimpleRuleBrain;
    use crate::plan::PlanBrainMode;
    use std::fs;

    fn write_agent_tree(root: &Path) {
        fs::create_dir_all(root.join(".agent/rules")).unwrap();
        fs::create_dir_all(root.join(".agent/skills/demo")).unwrap();
        fs::create_dir_all(root.join(".agent/tools")).unwrap();
        fs::write(root.join(".agent/rules/a.md"), "# rule a").unwrap();
        fs::write(
            root.join(".agent/skills/demo/task.json"),
            r#"{
                "id": "demo_skill",
                "summary": "demo",
                "steps": []
            }"#,
        )
        .unwrap();
        fs::write(root.join(".agent/skills/demo/SKILL.md"), "# Demo skill").unwrap();
        fs::write(
            root.join(".agent/tools/echo.json"),
            r#"{
                "name": "agent_echo",
                "spec": "args: { \"text\": \"...\" }",
                "command": "echo {text}"
            }"#,
        )
        .unwrap();
    }

    #[test]
    fn seed_builder_merges_agent_and_registers_plugins() {
        let root = std::env::temp_dir().join(format!("hs-seed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_agent_tree(&root);

        let prev_workspace = std::env::var("HARNESS_WORKSPACE").ok();
        let config = AgentProjectConfig::default();
        let builder = SeedBuilder::new()
            .task_registry(TaskRegistry::builtin())
            .merge_agent_project(&config, &root)
            .unwrap();

        let report = builder.agent_report().unwrap();
        assert_eq!(report.rules_files, 1);
        assert_eq!(report.skill_tasks, 1);
        assert_eq!(report.script_tools, 1);
        assert!(builder.task_registry_ref().get("demo_skill").is_some());
        assert!(!builder.blocks_ref().rules.is_empty());

        let react = builder.build(
            SimpleRuleBrain::new(),
            PlanBrainMode::rule(),
            ReActConfig::default(),
        );
        let names = react.registered_tool_names();
        assert!(
            names.iter().any(|n| n == "agent_echo"),
            "expected agent_echo in {names:?}"
        );

        match prev_workspace {
            Some(value) => unsafe { std::env::set_var("HARNESS_WORKSPACE", value) },
            None => unsafe { std::env::remove_var("HARNESS_WORKSPACE") },
        }
        let _ = fs::remove_dir_all(&root);
    }
}
