//! ツール実行: プラグイン [`Tool`] + [`ToolRegistry`] + [`ToolPack`]。

mod builtin;
mod pack;
mod registry;
mod traits;

pub use builtin::{resolve_in_workspace, workspace_root};
pub use pack::{apply_packs, default_packs, packs_from_names, ToolPack};
pub use registry::ToolRegistry;
pub use traits::{Tool, ToolContext};

use serde_json::{json, Value};

use crate::action::{Action, Observation};
use crate::brave_search::BraveSearchConfig;
use crate::runtime::RuntimeEnvironment;

pub const HELP_TEXT: &str = "\
利用可能なコマンド:
  help          このヘルプ
  time          現在時刻を取得（ツール経由）
  echo <text>   テキストをエコー（ツール経由）
  quit / exit   終了

LLM モードでは read_file / write_file / run_cmd などでコードの読み書き・実行ができます。

上記以外の入力は ReAct ループ（Thought → Action → Answer）で処理します。";

/// ツール実行ランタイム（invoke_id 採番 + レジストリ）。
#[derive(Debug)]
pub struct ToolRuntime {
    next_invoke_id: u64,
    registry: ToolRegistry,
    ctx: ToolContext,
}

impl Default for ToolRuntime {
    fn default() -> Self {
        Self::with_packs(RuntimeEnvironment::detect(), None, &default_packs(false))
    }
}

impl ToolRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_environment(env: RuntimeEnvironment) -> Self {
        Self::with_environment_and_brave(env, None)
    }

    pub fn with_environment_and_brave(
        env: RuntimeEnvironment,
        brave_search: Option<BraveSearchConfig>,
    ) -> Self {
        let include_web = brave_search.is_some();
        Self::with_packs(env, brave_search, &default_packs(include_web))
    }

    /// 指定パックだけを登録したランタイムを構築する。
    pub fn with_packs(
        env: RuntimeEnvironment,
        brave_search: Option<BraveSearchConfig>,
        packs: &[ToolPack],
    ) -> Self {
        let include_web = brave_search.is_some();
        let mut registry = ToolRegistry::new();
        if packs.is_empty() {
            apply_packs(&mut registry, &default_packs(include_web), include_web);
        } else if packs.contains(&ToolPack::Full) {
            ToolPack::Full.register_into(&mut registry, include_web);
        } else {
            apply_packs(&mut registry, packs, include_web);
        }
        Self {
            next_invoke_id: 0,
            registry,
            ctx: ToolContext::new(env, brave_search),
        }
    }

    pub fn from_registry(
        env: RuntimeEnvironment,
        brave_search: Option<BraveSearchConfig>,
        registry: ToolRegistry,
    ) -> Self {
        Self {
            next_invoke_id: 0,
            registry,
            ctx: ToolContext::new(env, brave_search),
        }
    }

    pub fn set_brave_search(&mut self, brave_search: Option<BraveSearchConfig>) {
        self.ctx.brave_search = brave_search.clone();
        if brave_search.is_some() && !self.registry.contains("web_search") {
            self.registry.register(Box::new(builtin::WebSearchTool));
        }
    }

    pub fn environment(&self) -> &RuntimeEnvironment {
        &self.ctx.env
    }

    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    pub fn catalog(&self) -> String {
        self.registry.format_catalog()
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.registry.contains(name)
    }

    /// ホストアプリから in-process ツールを追加する。
    pub fn register_plugin(&mut self, tool: Box<dyn Tool>) {
        self.registry.register(tool);
    }

    fn next_id(&mut self) -> u64 {
        self.next_invoke_id += 1;
        self.next_invoke_id
    }

    pub fn execute(&mut self, tool: &str, args: &Value) -> (u64, Observation) {
        let invoke_id = self.next_id();
        let observation = self.registry.execute(tool, invoke_id, args, &self.ctx);
        (invoke_id, observation)
    }
}

/// Action を実行して Observation を返す（invoke_id は Action 側を優先）。
pub fn execute_action(runtime: &mut ToolRuntime, action: &Action) -> Observation {
    let (_, mut obs) = runtime.execute(&action.tool, &action.args);
    obs.invoke_id = action.invoke_id;
    obs
}

/// 登録済みレジストリからカタログ文字列を生成（後方互換）。
pub fn format_tool_catalog(registry: &ToolRegistry) -> String {
    registry.format_catalog()
}

/// 全組み込みツールを登録したレジストリ（テスト・`ToolPack::Full` 用）。
pub fn full_builtin_registry(include_web: bool) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    ToolPack::Full.register_into(&mut registry, include_web);
    registry
}

pub fn echo_action(invoke_id: u64, message: &str) -> Action {
    Action::new(invoke_id, "echo", json!({ "message": message }))
}

pub fn time_action(invoke_id: u64) -> Action {
    Action::new(invoke_id, "time", json!({}))
}

pub fn list_dir_action(invoke_id: u64, path: &str) -> Action {
    Action::new(invoke_id, "list_dir", json!({ "path": path }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn list_dir_lists_cwd() {
        let mut rt = ToolRuntime::with_packs(
            RuntimeEnvironment::detect(),
            None,
            &[ToolPack::Coding],
        );
        let obs = rt.execute("list_dir", &json!({})).1;
        assert!(obs.ok);
        assert!(obs.output.contains("Cargo.toml"));
    }

    #[test]
    fn write_and_read_file_roundtrip() {
        let rel = "tmp/test_tool_roundtrip.txt";
        let abs = resolve_in_workspace(rel).unwrap();
        let _ = fs::remove_file(&abs);

        let mut rt = ToolRuntime::with_packs(
            RuntimeEnvironment::detect(),
            None,
            &[ToolPack::Coding],
        );
        let write = rt
            .execute(
                "write_file",
                &json!({ "path": rel, "content": "fn main() {}\n" }),
            )
            .1;
        assert!(write.ok, "{}", write.output);

        let read = rt.execute("read_file", &json!({ "path": rel })).1;
        assert!(read.ok);
        assert_eq!(read.output, "fn main() {}\n");

        let _ = fs::remove_file(&abs);
    }

    #[test]
    fn rejects_path_outside_workspace() {
        assert!(resolve_in_workspace("..").is_err());
        assert!(resolve_in_workspace("tmp/../../outside.txt").is_err());
    }

    #[test]
    fn grep_finds_in_src() {
        let mut rt = ToolRuntime::with_packs(
            RuntimeEnvironment::detect(),
            None,
            &[ToolPack::Coding],
        );
        let obs = rt
            .execute(
                "grep",
                &json!({
                    "pattern": "pub struct ReActLoop",
                    "path": "src",
                    "glob": "*.rs",
                    "max_results": 5
                }),
            )
            .1;
        assert!(obs.ok, "{}", obs.output);
        assert!(obs.output.contains("react.rs"));
    }

    #[test]
    fn run_cmd_echo() {
        let mut rt = ToolRuntime::with_packs(
            RuntimeEnvironment::detect(),
            None,
            &[ToolPack::Coding],
        );
        let command = "echo hello_cmd";
        let obs = rt.execute("run_cmd", &json!({ "command": command })).1;
        assert!(obs.ok, "{}", obs.output);
        assert!(obs.output.contains("hello_cmd"));
    }

    #[test]
    fn basic_pack_unknown_tool() {
        let mut rt = ToolRuntime::with_packs(
            RuntimeEnvironment::detect(),
            None,
            &[ToolPack::Basic],
        );
        let obs = rt.execute("grep", &json!({ "pattern": "x" })).1;
        assert!(!obs.ok);
        assert!(obs.output.contains("unknown tool"));
    }
}
