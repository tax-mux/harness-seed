//! 機能塊タスク: **必須実行メソッド**と**実行順序**の定義（`tasks/*.json`）。

mod audit;
mod driver;
mod policy;
mod registry;
mod spec;

pub use audit::{audit_trace, expected_args, StepAudit, TaskExecutionAudit};
pub use driver::{StepDriverError, StepDriverResult};
pub use policy::{SubtaskToolPolicy, ToolPolicySpec};
pub use registry::{extract_reference_uid, TaskLoadError, TaskRegistry};
pub use spec::{
    apply_template, apply_template_value, ExecStep, MissionRenderContext, TaskDefinition,
    TaskError,
};
