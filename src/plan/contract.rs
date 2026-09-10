//! 計画層のデータ契約（内部3層）:
//! - **INPUT**（read）— ホストが固定。LLM は変更しない。
//! - **PROCEDURE**（PlanArtifact subtasks）— LLM が設計する唯一の可変部分。
//! - **OUTPUT**（write）— ホストが固定。LLM は変更しない。
//!
//! ドメイン語彙（メール / Web / コーディング等）はホストが `input_layer` /
//! `output_layer` / `enforce` に載せる。コアは形だけを扱う。

use std::fmt;
use std::sync::Arc;

use super::PlanArtifact;

/// ホストが渡す計画正規化フック。
pub type PlanEnforceFn = Arc<dyn Fn(&mut PlanArtifact) + Send + Sync>;

/// 1 ターン分の read / write 契約（LLM に推測させない）。
#[derive(Clone)]
pub struct PlanDataContract {
    /// 挨拶・雑談などツール不要ターン。
    pub skip_execution: bool,
    /// Planner 向け INPUT 層テキスト（ホスト定義）。
    pub input_layer: String,
    /// Planner 向け OUTPUT 層テキスト（ホスト定義）。
    pub output_layer: String,
    /// 手順層で使ってよい task id のヒント（ホスト定義）。
    pub procedure_hint: String,
    /// 計画カタログから除外する task id。
    pub excluded_task_ids: Vec<String>,
    /// タスク params へ注入する参照 id（`uid` キー）。不要なら `None`。
    pub reference_id: Option<i64>,
    /// 参照証拠が既にプロンプト内にある等で、参照 id からの自動取得を抑止する。
    pub blocks_reference_fetch: bool,
    enforce: Option<PlanEnforceFn>,
}

impl PlanDataContract {
    /// ホスト定義の境界だけを持つ契約。
    pub fn new(
        input_layer: impl Into<String>,
        output_layer: impl Into<String>,
        procedure_hint: impl Into<String>,
    ) -> Self {
        Self {
            skip_execution: false,
            input_layer: input_layer.into(),
            output_layer: output_layer.into(),
            procedure_hint: procedure_hint.into(),
            excluded_task_ids: Vec::new(),
            reference_id: None,
            blocks_reference_fetch: false,
            enforce: None,
        }
    }

    /// 挨拶等: 手順層ごとスキップ（LLM 計画ループも不要）。
    pub fn trivial_chat() -> Self {
        Self {
            skip_execution: true,
            input_layer: "read: user_message (prompt only)".into(),
            output_layer: "chat_only (final answer in chat)".into(),
            procedure_hint: "skip_execution only".into(),
            excluded_task_ids: Vec::new(),
            reference_id: None,
            blocks_reference_fetch: true,
            enforce: None,
        }
    }

    pub fn with_excluded_task_ids(mut self, ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.excluded_task_ids = ids.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_reference_id(mut self, id: Option<i64>) -> Self {
        self.reference_id = id;
        self
    }

    pub fn with_blocks_reference_fetch(mut self, blocks: bool) -> Self {
        self.blocks_reference_fetch = blocks;
        self
    }

    /// 契約違反の plan をホスト側ルールで正規化するフック。
    pub fn with_enforce(mut self, f: impl Fn(&mut PlanArtifact) + Send + Sync + 'static) -> Self {
        self.enforce = Some(Arc::new(f));
        self
    }

    pub fn skip_plan_layer(&self) -> bool {
        self.skip_execution
    }

    /// 計画層内部: LLM が埋める手順層の制約。
    pub fn format_procedure_layer(&self) -> String {
        if self.skip_execution {
            return "Emit plan JSON with skip_execution: true and empty steps.".into();
        }
        format!(
            "Hard instruction: read ONLY from INPUT and write ONLY to OUTPUT; plan ONLY the in-between procedure.\n\
             Plan subtasks (procedure) that connect INPUT read → OUTPUT write.\n\
             Allowed task ids: {}\n\
             Output JSON shape:\n\
             {{\"input\":[\"...\"],\"steps\":[{{\"id\":1,\"task\":\"...\",\"params\":{{}},\"goal\":\"...\",\"done_when\":\"...\"}}],\"output\":\"...\",\"skip_execution\":false}}\n\
             Do not add steps that read/write elsewhere.",
            self.procedure_hint
        )
    }

    /// 計画 LLM 向け: 入力層 → 手順層(LLM) → 出力層 の3層構造。
    pub fn format_for_planner(&self) -> String {
        [
            "--- Plan layer: INPUT → PROCEDURE (you) → OUTPUT ---".into(),
            String::new(),
            "[INPUT — fixed, do not change]".into(),
            self.input_layer.clone(),
            String::new(),
            "[OUTPUT — fixed, do not change]".into(),
            self.output_layer.clone(),
            String::new(),
            "[PROCEDURE — your PlanArtifact subtasks]".into(),
            self.format_procedure_layer(),
        ]
        .join("\n")
    }

    /// 契約違反の plan を正規化する（`resolve_plan` から呼ぶ）。
    pub fn enforce_plan(&self, plan: &mut PlanArtifact) {
        if self.skip_execution {
            plan.skip_execution = true;
            plan.subtasks.clear();
            return;
        }
        if plan.skip_execution {
            return;
        }
        if let Some(f) = &self.enforce {
            f(plan);
        }
    }
}

impl fmt::Debug for PlanDataContract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlanDataContract")
            .field("skip_execution", &self.skip_execution)
            .field("input_layer", &self.input_layer)
            .field("output_layer", &self.output_layer)
            .field("procedure_hint", &self.procedure_hint)
            .field("excluded_task_ids", &self.excluded_task_ids)
            .field("reference_id", &self.reference_id)
            .field("blocks_reference_fetch", &self.blocks_reference_fetch)
            .field("enforce", &self.enforce.as_ref().map(|_| "<fn>"))
            .finish()
    }
}

impl PartialEq for PlanDataContract {
    fn eq(&self, other: &Self) -> bool {
        self.skip_execution == other.skip_execution
            && self.input_layer == other.input_layer
            && self.output_layer == other.output_layer
            && self.procedure_hint == other.procedure_hint
            && self.excluded_task_ids == other.excluded_task_ids
            && self.reference_id == other.reference_id
            && self.blocks_reference_fetch == other.blocks_reference_fetch
    }
}

impl Eq for PlanDataContract {}
