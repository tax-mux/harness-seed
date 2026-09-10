//! Harness パース — Planner テキスト（作業指示書）→ 内部 JSON [`HarnessState`]。

mod parse;
mod reference;
mod state;

pub use parse::{parse_harness, parse_harness_strict, HarnessParseError};
pub use reference::{format_references_for_prompt, HarnessMailRefKind, HarnessReference};
pub use state::{HarnessState, HarnessStatus};
