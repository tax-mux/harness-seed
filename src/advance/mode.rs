//! 推進ループの入り方（`react.advance.mode`）。

use std::fmt;

/// 推進ループをいつ入口にするか。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdvanceMode {
    /// 推進ループに入らない。
    #[default]
    Off,
    /// 計画後の判定なしで推進ループへ入る（旧 `enabled: true`）。
    Always,
    /// 計画を 1 回走らせ、`PlanArtifact` の形で昇格する。
    FromPlan,
}

impl AdvanceMode {
    /// 設定文字列を解釈する。未知値は `Off`。
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "false" | "disabled" => Self::Off,
            "always" | "true" | "on" | "enabled" => Self::Always,
            "from_plan" | "from-plan" | "auto" => Self::FromPlan,
            _ => Self::Off,
        }
    }

    /// 無条件に推進ループへ入るか。
    pub fn is_always(self) -> bool {
        matches!(self, Self::Always)
    }
}

impl fmt::Display for AdvanceMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::Always => "always",
            Self::FromPlan => "from_plan",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_aliases() {
        assert_eq!(AdvanceMode::parse("always"), AdvanceMode::Always);
        assert_eq!(AdvanceMode::parse("true"), AdvanceMode::Always);
        assert_eq!(AdvanceMode::parse("from_plan"), AdvanceMode::FromPlan);
        assert_eq!(AdvanceMode::parse("auto"), AdvanceMode::FromPlan);
        assert_eq!(AdvanceMode::parse("off"), AdvanceMode::Off);
        assert_eq!(AdvanceMode::parse("nope"), AdvanceMode::Off);
    }
}
