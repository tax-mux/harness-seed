//! `agent_dir` 配下の rules / skills / tools を読み込む。

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::context::{ContextError, PromptBlocks};
use crate::tasks::{TaskDefinition, TaskLoadError, TaskRegistry};
use crate::tool::Tool;

use super::config_agent::{AgentProjectConfig, ResolvedAgentPaths};
use super::script_tool::{ScriptTool, ScriptToolDefinition};

#[derive(Debug, Clone, Default)]
pub struct AgentLoadReport {
    pub workspace: PathBuf,
    pub agent_dir: PathBuf,
    pub rules_files: usize,
    pub skill_tasks: usize,
    pub skill_docs: usize,
    pub script_tools: usize,
}

#[derive(Debug)]
pub enum AgentLoadError {
    Config(super::config_agent::AgentConfigError),
    Context(ContextError),
    Tasks(TaskLoadError),
    Tools { path: PathBuf, reason: String },
}

impl fmt::Display for AgentLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => write!(f, "{e}"),
            Self::Context(e) => write!(f, "{e}"),
            Self::Tasks(e) => write!(f, "{e}"),
            Self::Tools { path, reason } => {
                write!(f, "tool {}: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for AgentLoadError {}

impl From<super::config_agent::AgentConfigError> for AgentLoadError {
    fn from(value: super::config_agent::AgentConfigError) -> Self {
        Self::Config(value)
    }
}

impl From<ContextError> for AgentLoadError {
    fn from(value: ContextError) -> Self {
        Self::Context(value)
    }
}

impl From<TaskLoadError> for AgentLoadError {
    fn from(value: TaskLoadError) -> Self {
        Self::Tasks(value)
    }
}

/// 解決済みパスから rules / skills / tools を読み込む。
pub fn load_agent_assets(
    paths: &ResolvedAgentPaths,
    blocks: &mut PromptBlocks,
    task_registry: &mut TaskRegistry,
) -> Result<(AgentLoadReport, Vec<Box<dyn Tool>>), AgentLoadError> {
    let mut report = AgentLoadReport {
        workspace: paths.workspace.clone(),
        agent_dir: paths.agent_dir.clone(),
        ..Default::default()
    };

    let rules_dir = paths.agent_dir.join("rules");
    if rules_dir.is_dir() {
        let files = collect_md_files(&rules_dir)?;
        report.rules_files = files.len();
        blocks.load_rules_from_paths(&files)?;
    }

    let skills_dir = paths.agent_dir.join("skills");
    if skills_dir.is_dir() {
        load_skills(&skills_dir, blocks, task_registry, &mut report)?;
    }

    let tools_dir = paths.agent_dir.join("tools");
    let script_tools = if tools_dir.is_dir() {
        load_script_tools(&tools_dir)?
    } else {
        Vec::new()
    };
    report.script_tools = script_tools.len();

    if let Some(manifest_path) = blocks.context_manifest_path.clone() {
        if let Err(e) = crate::context_manifest::note_manifest_available(&manifest_path, blocks) {
            eprintln!("context manifest: {e}");
        }
    }

    Ok((report, script_tools))
}

/// ファイルから rules / tasks / script tools を集める（呼び出し側が登録する旧経路）。
///
/// 埋め込みホストは [`crate::seed::SeedBuilder::merge_agent_project`] を使うこと。
pub fn apply_agent_project(
    config: &AgentProjectConfig,
    base_dir: &Path,
    blocks: &mut PromptBlocks,
    task_registry: &mut TaskRegistry,
) -> Result<(AgentLoadReport, Vec<Box<dyn Tool>>), AgentLoadError> {
    let paths = config.resolve_paths(base_dir)?;
    super::config_agent::apply_workspace_env(&paths.workspace);
    blocks.context_manifest_path = config.resolved_context_manifest_path(base_dir)?;
    load_agent_assets(&paths, blocks, task_registry)
}

fn load_skills(
    skills_dir: &Path,
    blocks: &mut PromptBlocks,
    task_registry: &mut TaskRegistry,
    report: &mut AgentLoadReport,
) -> Result<(), AgentLoadError> {
    let mut child_dirs: Vec<PathBuf> = fs::read_dir(skills_dir)
        .map_err(|source| TaskLoadError::Read {
            path: skills_dir.to_path_buf(),
            source,
        })?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    child_dirs.sort();

    for dir in child_dirs {
        let task_path = dir.join("task.json");
        if task_path.is_file() {
            let text = fs::read_to_string(&task_path).map_err(|source| TaskLoadError::Read {
                path: task_path.clone(),
                source,
            })?;
            let def: TaskDefinition =
                serde_json::from_str(&text).map_err(|source| TaskLoadError::Parse {
                    path: task_path.clone(),
                    source,
                })?;
            task_registry.register(def).map_err(|e| TaskLoadError::Invalid {
                path: task_path,
                reason: e.to_string(),
            })?;
            report.skill_tasks += 1;
        }

        for doc_name in ["SKILL.md", "skill.prompt.md"] {
            let doc_path = dir.join(doc_name);
            if doc_path.is_file() {
                let text = fs::read_to_string(&doc_path).map_err(|source| ContextError::Read {
                    path: doc_path.clone(),
                    source,
                })?;
                let label = doc_path.display();
                blocks.push_rule(format!("--- skill: {label} ---\n{text}"));
                report.skill_docs += 1;
            }
        }
    }

    let mut top_json: Vec<PathBuf> = fs::read_dir(skills_dir)
        .map_err(|source| TaskLoadError::Read {
            path: skills_dir.to_path_buf(),
            source,
        })?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "json"))
        .collect();
    top_json.sort();
    if !top_json.is_empty() {
        task_registry.load_dir(skills_dir)?;
        report.skill_tasks += top_json.len();
    }

    Ok(())
}

fn load_script_tools(tools_dir: &Path) -> Result<Vec<Box<dyn Tool>>, AgentLoadError> {
    let mut paths: Vec<PathBuf> = fs::read_dir(tools_dir)
        .map_err(|source| AgentLoadError::Tools {
            path: tools_dir.to_path_buf(),
            reason: source.to_string(),
        })?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();

    let mut out: Vec<Box<dyn Tool>> = Vec::new();
    for path in paths {
        let text = fs::read_to_string(&path).map_err(|source| AgentLoadError::Tools {
            path: path.clone(),
            reason: source.to_string(),
        })?;
        let mut def: ScriptToolDefinition =
            serde_json::from_str(&text).map_err(|source| AgentLoadError::Tools {
                path: path.clone(),
                reason: source.to_string(),
            })?;
        if def.spec.trim().is_empty() {
            def.spec = def.resolved_spec();
        }
        let tool = ScriptTool::from_definition(def).map_err(|reason| AgentLoadError::Tools {
            path: path.clone(),
            reason,
        })?;
        out.push(Box::new(tool));
    }
    Ok(out)
}

fn collect_md_files(dir: &Path) -> Result<Vec<PathBuf>, ContextError> {
    let mut out = Vec::new();
    collect_md_files_rec(dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_md_files_rec(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), ContextError> {
    for entry in fs::read_dir(dir).map_err(|source| ContextError::Read {
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| ContextError::Read {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_md_files_rec(&path, out)?;
        } else if path.is_file() && path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::PromptBlocks;
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
    fn load_agent_assets_reads_rules_skills_tools() {
        let root = std::env::temp_dir().join(format!("hs-agent-load-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_agent_tree(&root);

        let prev_workspace = std::env::var("HARNESS_WORKSPACE").ok();
        let config = AgentProjectConfig::default();
        let mut blocks = PromptBlocks::new();
        let mut registry = TaskRegistry::builtin();
        let (report, tools) =
            apply_agent_project(&config, &root, &mut blocks, &mut registry).unwrap();

        assert_eq!(report.rules_files, 1);
        assert_eq!(report.skill_tasks, 1);
        assert_eq!(report.skill_docs, 1);
        assert_eq!(tools.len(), 1);
        assert!(registry.get("demo_skill").is_some());
        assert!(!blocks.rules.is_empty());

        match prev_workspace {
            Some(value) => unsafe { std::env::set_var("HARNESS_WORKSPACE", value) },
            None => unsafe { std::env::remove_var("HARNESS_WORKSPACE") },
        }
        let _ = fs::remove_dir_all(&root);
    }
}
