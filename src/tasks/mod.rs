//! 機能塊タスク: **必須実行メソッド**と**実行順序**の定義（`tasks/*.json`）。

mod audit;
mod driver;
mod registry;
mod spec;

pub use audit::{audit_trace, expected_args, StepAudit, TaskExecutionAudit};
pub use driver::{StepDriverError, StepDriverResult};
pub use registry::{TaskLoadError, TaskRegistry};
pub use spec::{
    apply_template, apply_template_value, ExecStep, MissionRenderContext, TaskDefinition,
    TaskError,
};
