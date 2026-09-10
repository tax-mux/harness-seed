//! ツールパッケージ（基本セット / コーディング拡張など）。

use super::builtin::{
    EchoTool, GrepTool, ListDirTool, ReadFileTool, RunCmdTool, TimeTool, WebSearchTool,
    WriteFileTool,
};
use super::registry::ToolRegistry;
/// あらかじめ定義したツール束。複数指定で合成できる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolPack {
    /// `echo`, `time`
    Basic,
    /// `list_dir`, `grep`, `read_file`, `write_file`, `run_cmd`
    Coding,
    /// `web_search`（Brave API キーがあるときのみ登録）
    WebSearch,
    /// Basic + Coding + WebSearch（キーあり時）
    Full,
}

impl ToolPack {
    pub const ALL: &'static [ToolPack] = &[ToolPack::Basic, ToolPack::Coding, ToolPack::WebSearch];

    pub fn id(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Coding => "coding",
            Self::WebSearch => "web_search",
            Self::Full => "full",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "basic" => Some(Self::Basic),
            "coding" => Some(Self::Coding),
            "web" | "web_search" => Some(Self::WebSearch),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    pub fn register_into(self, registry: &mut ToolRegistry, include_web: bool) {
        match self {
            Self::Basic => {
                registry.register(Box::new(EchoTool));
                registry.register(Box::new(TimeTool));
            }
            Self::Coding => {
                registry.register(Box::new(ListDirTool));
                registry.register(Box::new(GrepTool));
                registry.register(Box::new(ReadFileTool));
                registry.register(Box::new(WriteFileTool));
                registry.register(Box::new(RunCmdTool));
            }
            Self::WebSearch => {
                if include_web {
                    registry.register(Box::new(WebSearchTool));
                }
            }
            Self::Full => {
                Self::Basic.register_into(registry, include_web);
                Self::Coding.register_into(registry, include_web);
                Self::WebSearch.register_into(registry, include_web);
            }
        }
    }
}

/// 複数パックをレジストリに適用する（同名は後勝ち）。
pub fn apply_packs(registry: &mut ToolRegistry, packs: &[ToolPack], include_web: bool) {
    for pack in packs {
        pack.register_into(registry, include_web);
    }
}

/// 設定文字列列を [`ToolPack`] に変換（未知名は無視）。
pub fn packs_from_names(names: &[String]) -> Vec<ToolPack> {
    names.iter().filter_map(|n| ToolPack::parse(n)).collect()
}

/// 既定: `basic` + `coding`、Brave キーがあれば `web_search` も追加。
pub fn default_packs(include_web: bool) -> Vec<ToolPack> {
    let mut packs = vec![ToolPack::Basic, ToolPack::Coding];
    if include_web {
        packs.push(ToolPack::WebSearch);
    }
    packs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_excludes_grep() {
        let mut reg = ToolRegistry::new();
        ToolPack::Basic.register_into(&mut reg, false);
        assert!(reg.contains("echo"));
        assert!(!reg.contains("grep"));
    }

    #[test]
    fn coding_includes_grep() {
        let mut reg = ToolRegistry::new();
        ToolPack::Coding.register_into(&mut reg, false);
        assert!(reg.contains("grep"));
        assert!(reg.contains("run_cmd"));
    }

    #[test]
    fn full_without_web_when_disabled() {
        let mut reg = ToolRegistry::new();
        ToolPack::Full.register_into(&mut reg, false);
        assert!(reg.contains("read_file"));
        assert!(!reg.contains("web_search"));
    }
}
