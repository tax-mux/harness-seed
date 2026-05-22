//! プロンプト用コンテキストブロックの組み立て（外部から rules / recalled を差し込み可能）。

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::action::TurnTrace;
use crate::context_metrics::format_messages_body;
use crate::llm::ChatMessage;
use crate::session::SessionMemory;
/// ReAct ループ用の固定 system 指示（ツール一覧は [`PromptBlocks::tool_catalog`] で付与）。
pub const REACT_SYSTEM_CORE: &str = r#"You are an agent in a ReAct loop. Reply with ONE JSON object only (no markdown).

Schema:
- {"step":"thought","content":"<reasoning>"}
- {"step":"action","tool":"<name>","args":{...}}
- {"step":"answer","content":"<final reply to user>"}

Rules:
- Use only tools listed in the Tool catalog below (exact names and args).
- If the user needs a tool, return action (not answer yet).
- After observations appear in the trace, use them and return answer when done.
- For coding tasks in this repo: grep or list_dir → read_file → write_file → run_cmd (e.g. cargo check) as needed.
- For simple greetings, you may answer directly without tools.
- All filesystem paths must stay inside the project workspace.
"#;

/// Brave Search が有効なときだけ system に追記する Web 検索 ReAct 指針。
pub const REACT_WEB_SEARCH_GUIDANCE: &str = r#"
Web search ReAct (Brave Search API is enabled):
- Use web_search when the user asks about current events, external APIs, documentation not in the repo, or facts you cannot verify from workspace files alone.
- Typical flow: thought → web_search with a short focused query (and optional count) → read observations → answer citing titles/URLs from results.
- You may combine web_search with local tools: e.g. web_search then read_file to compare upstream docs with this codebase.
- Do not use web_search for pure local edits, grep-only exploration, or greetings unless the user explicitly wants web lookup.
- If web_search fails (missing key / API error), say so in the answer and continue with local tools only if still useful.
"#;

/// セッションをまたいで保持するプロンプトブロック（rules / recalled など）。
#[derive(Debug, Clone)]
pub struct PromptBlocks {
    /// 外部ルール（`.md` や設定 `rules_paths` から読み込み）。
    pub rules: Vec<String>,
    /// 検索・RAG など外部から注入する追記コンテキスト。
    pub recalled: Vec<String>,
    /// system メッセージ末尾に追加する任意テキスト。
    pub system_extra: String,
    /// 起動時に検出した OS / シェル（`run_cmd` とプロンプトで共有）。
    pub runtime: crate::runtime::RuntimeEnvironment,
    /// `tools.brave_search` が有効なとき true — [`REACT_WEB_SEARCH_GUIDANCE`] を system に載せる。
    pub web_search_enabled: bool,
    /// 登録済みツールの `Tool catalog` ブロック（[`ToolRuntime::catalog`] から設定）。
    pub tool_catalog: String,
}

impl Default for PromptBlocks {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            recalled: Vec::new(),
            system_extra: String::new(),
            runtime: crate::runtime::RuntimeEnvironment::detect(),
            web_search_enabled: false,
            tool_catalog: crate::tool::format_tool_catalog(&crate::tool::full_builtin_registry(
                false,
            )),
        }
    }
}

impl PromptBlocks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_rule(&mut self, text: impl Into<String>) {
        let t = text.into();
        if !t.trim().is_empty() {
            self.rules.push(t);
        }
    }

    pub fn push_recalled(&mut self, text: impl Into<String>) {
        let t = text.into();
        if !t.trim().is_empty() {
            self.recalled.push(t);
        }
    }

    pub fn clear_rules(&mut self) {
        self.rules.clear();
    }

    pub fn clear_recalled(&mut self) {
        self.recalled.clear();
    }

    /// パス（ファイルまたはディレクトリ）から rules を読み込んで追記する。
    pub fn load_rules_from_paths(&mut self, paths: &[PathBuf]) -> Result<(), ContextError> {
        for path in paths {
            self.load_rules_from_path(path)?;
        }
        Ok(())
    }

    fn load_rules_from_path(&mut self, path: &Path) -> Result<(), ContextError> {
        if !path.exists() {
            return Err(ContextError::NotFound {
                path: path.to_path_buf(),
            });
        }
        if path.is_file() {
            let text = fs::read_to_string(path).map_err(|source| ContextError::Read {
                path: path.to_path_buf(),
                source,
            })?;
            let label = path.display();
            self.push_rule(format!("--- rules: {label} ---\n{text}"));
            return Ok(());
        }
        if path.is_dir() {
            let mut entries: Vec<PathBuf> = fs::read_dir(path)
                .map_err(|source| ContextError::Read {
                    path: path.to_path_buf(),
                    source,
                })?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "md"))
                .collect();
            entries.sort();
            for file in entries {
                let text = fs::read_to_string(&file).map_err(|source| ContextError::Read {
                    path: file.clone(),
                    source,
                })?;
                let label = file.display();
                self.push_rule(format!("--- rules: {label} ---\n{text}"));
            }
        }
        Ok(())
    }
}

/// 1 回の `decide` 呼び出し用のプロンプト文脈。
#[derive(Debug, Clone, Copy)]
pub struct TurnPromptContext<'a> {
    pub blocks: &'a PromptBlocks,
    pub user_input: &'a str,
    pub trace: &'a TurnTrace,
    pub session: &'a SessionMemory,
}

impl<'a> TurnPromptContext<'a> {
    pub fn new(
        blocks: &'a PromptBlocks,
        user_input: &'a str,
        trace: &'a TurnTrace,
        session: &'a SessionMemory,
    ) -> Self {
        Self {
            blocks,
            user_input,
            trace,
            session,
        }
    }

    /// LLM コネクタへ渡す `system` + `user` メッセージ列。
    pub fn render(&self) -> Vec<ChatMessage> {
        vec![
            ChatMessage::system(self.system_content()),
            ChatMessage::user(self.user_content()),
        ]
    }

    fn system_content(&self) -> String {
        let mut out = String::from(REACT_SYSTEM_CORE);
        out.push('\n');
        if !self.blocks.tool_catalog.is_empty() {
            out.push_str(&self.blocks.tool_catalog);
            if !self.blocks.tool_catalog.ends_with('\n') {
                out.push('\n');
            }
        }
        if self.blocks.web_search_enabled {
            out.push_str(REACT_WEB_SEARCH_GUIDANCE);
        }
        if !self.blocks.rules.is_empty() {
            out.push_str("\n\nAdditional rules:\n");
            for (i, rule) in self.blocks.rules.iter().enumerate() {
                out.push_str(&format!("\n[rule {}]\n{rule}\n", i + 1));
            }
        }
        if !self.blocks.recalled.is_empty() {
            out.push_str("\n\nRecalled context:\n");
            for (i, chunk) in self.blocks.recalled.iter().enumerate() {
                out.push_str(&format!("\n[recalled {}]\n{chunk}\n", i + 1));
            }
        }
        if !self.blocks.system_extra.is_empty() {
            out.push_str("\n\n");
            out.push_str(&self.blocks.system_extra);
        }
        out.push_str("\n\nExecution environment:\n");
        out.push_str(&self.blocks.runtime.prompt_hint());
        out
    }

    fn user_content(&self) -> String {
        let previous = self.session.format_for_prompt();
        let previous_block = if previous.is_empty() {
            String::new()
        } else {
            format!("{previous}\n")
        };
        let trace_text = format_trace(self.trace);
        format!(
            "{previous_block}User input:\n{}\n\nTurn trace so far:\n{trace_text}\n\nNext step JSON:",
            self.user_input
        )
    }
}

/// ループ 1 ステップ分のプロンプト本文を stderr に出す（`react.show_prompt`）。
pub fn eprintln_step_prompt(label: &str, step: usize, body: &str) {
    eprintln!("\n--- [{label}] prompt step {step} ---\n{body}--- end prompt ---\n");
}

/// [`TurnPromptContext::render`] をログ用テキストにする。
pub fn format_prompt_messages(messages: &[ChatMessage]) -> String {
    format_messages_body(messages)
}

/// 計画層ルール頭脳向けのプロンプトプレビュー（LLM 未使用時）。
pub fn format_plan_rule_prompt_preview(ctx: &TurnPromptContext<'_>) -> String {
    let previous = ctx.session.format_for_prompt();
    let previous_block = if previous.is_empty() {
        String::new()
    } else {
        format!("{previous}\n")
    };
    let trace_text = format_trace(ctx.trace);
    format!(
        "system: <rule plan brain — LLM not used>\nuser: {previous_block}Plan request:\n{}\n\nPlan trace so far:\n{trace_text}\n\nNext plan step JSON:",
        ctx.user_input
    )
}

pub fn format_trace(trace: &TurnTrace) -> String {
    let mut trace_text = String::new();
    for (i, t) in trace.thoughts.iter().enumerate() {
        trace_text.push_str(&format!("[thought {i}] {t}\n"));
    }
    for action in &trace.actions {
        trace_text.push_str(&format!(
            "[action {}] {} {}\n",
            action.invoke_id, action.tool, action.args
        ));
    }
    for obs in &trace.observations {
        let status = if obs.ok { "ok" } else { "err" };
        trace_text.push_str(&format!(
            "[observation {}] {status}: {}\n",
            obs.invoke_id, obs.output
        ));
    }
    if trace_text.is_empty() {
        trace_text.push_str("(empty trace — first step this turn)\n");
    }
    trace_text
}

#[derive(Debug)]
pub enum ContextError {
    NotFound { path: PathBuf },
    Read { path: PathBuf, source: io::Error },
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => write!(f, "rules path not found: {}", path.display()),
            Self::Read { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ContextError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Observation;
    use std::fs;

    #[test]
    fn plan_rule_prompt_preview_includes_plan_request() {
        let blocks = PromptBlocks::default();
        let trace = TurnTrace::default();
        let session = SessionMemory::default();
        let ctx = TurnPromptContext::new(&blocks, "list src", &trace, &session);
        let body = format_plan_rule_prompt_preview(&ctx);
        assert!(body.contains("Plan request:"));
        assert!(body.contains("list src"));
    }

    #[test]
    fn render_includes_previous_turns_and_user_input() {
        let mut session = SessionMemory::new(4);
        session.push_turn("first", "one");
        let blocks = PromptBlocks::default();
        let trace = TurnTrace::default();
        let ctx = TurnPromptContext::new(&blocks, "second", &trace, &session);
        let messages = ctx.render();
        let user = messages
            .iter()
            .find(|m| m.role == "user")
            .expect("user");
        assert!(user.content.contains("Previous turns:"));
        assert!(user.content.contains("User: first"));
        assert!(user.content.contains("User input:\nsecond"));
    }

    #[test]
    fn render_includes_web_search_guidance_when_enabled() {
        let mut blocks = PromptBlocks::default();
        blocks.web_search_enabled = true;
        let trace = TurnTrace::default();
        let session = SessionMemory::default();
        let ctx = TurnPromptContext::new(&blocks, "latest Rust news", &trace, &session);
        let system = ctx
            .render()
            .into_iter()
            .find(|m| m.role == "system")
            .expect("system");
        assert!(system.content.contains("web_search"));
        assert!(system.content.contains("Web search ReAct"));
    }

    #[test]
    fn render_omits_web_search_guidance_when_disabled() {
        let blocks = PromptBlocks::default();
        let trace = TurnTrace::default();
        let session = SessionMemory::default();
        let ctx = TurnPromptContext::new(&blocks, "hi", &trace, &session);
        let system = ctx
            .render()
            .into_iter()
            .find(|m| m.role == "system")
            .expect("system");
        assert!(!system.content.contains("Web search ReAct"));
    }

    #[test]
    fn render_includes_execution_environment() {
        let blocks = PromptBlocks::default();
        let trace = TurnTrace::default();
        let session = SessionMemory::default();
        let ctx = TurnPromptContext::new(&blocks, "hi", &trace, &session);
        let system = ctx
            .render()
            .into_iter()
            .find(|m| m.role == "system")
            .expect("system");
        assert!(system.content.contains("Execution environment:"));
        assert!(system.content.contains(&blocks.runtime.os));
    }

    #[test]
    fn render_includes_rules_and_recalled_in_system() {
        let mut blocks = PromptBlocks::new();
        blocks.push_rule("always be polite");
        blocks.push_recalled("doc snippet");
        let trace = TurnTrace::default();
        let session = SessionMemory::default();
        let ctx = TurnPromptContext::new(&blocks, "hi", &trace, &session);
        let system = ctx
            .render()
            .into_iter()
            .find(|m| m.role == "system")
            .expect("system");
        assert!(system.content.contains("Additional rules:"));
        assert!(system.content.contains("always be polite"));
        assert!(system.content.contains("Recalled context:"));
        assert!(system.content.contains("doc snippet"));
    }

    #[test]
    fn format_trace_includes_observation() {
        let mut trace = TurnTrace::default();
        trace.push_observation(Observation::success(1, "ok out"));
        let text = format_trace(&trace);
        assert!(text.contains("[observation 1] ok:"));
        assert!(text.contains("ok out"));
    }

    #[test]
    fn load_rules_from_md_file() {
        let dir = std::env::temp_dir().join("harness_seed_ctx_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("extra.md");
        fs::write(&file, "# Rule\nDo X.").unwrap();

        let mut blocks = PromptBlocks::new();
        blocks.load_rules_from_paths(&[file]).unwrap();
        assert_eq!(blocks.rules.len(), 1);
        assert!(blocks.rules[0].contains("Do X."));

        let _ = fs::remove_dir_all(&dir);
    }
}
