//! 機能塊タスク: **必須実行メソッド**と**実行順序**の定義（`tasks/*.json`）。

mod audit;
mod driver;
mod policy;
mod registry;
mod spec;

pub use audit::{
    audit_trace, audit_trace_with_mode, expected_args, ArgAuditMode, StepAudit, TaskExecutionAudit,
};
pub use driver::{StepDriverError, StepDriverResult};
pub use policy::{SubtaskToolPolicy, ToolPolicySpec};
pub use registry::{TaskLoadError, TaskRegistry};
pub use spec::{
    apply_template, apply_template_value, ContextManifestSpec, ExecStep, MissionRenderContext,
    TaskDefinition, TaskError,
};
