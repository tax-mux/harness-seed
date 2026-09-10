MUSE-Autoskill Integration for harness-seed
文脈: ByteDance MUSE-Autoskill (arXiv:2605.27366) をDtmOyajiのharness-seedに統合し、スキル（プロンプト・指示）を「消費物」から「進化する資産」へ昇格させる設計書。

📌 前置き：現状の問題点
改良点ログより（Triage Mail + harness-seed）:

改良#10: 短文テンプレートの「登録・編集機能」が手動ベース
改良#16: バージョンアップ時に「指示定型文が消失する」実績あり
改良#19: i18n で「言語ごとに別セット管理」が必要だが、スキル共有メカニズムない
改良#21: 代理店向けライセンス管理で「スキルの移植」をサポートしたい

MUSEの解法:
スキル → 長生きする、経験を積む、テスト可能な資産 へ

🎯 目標アーキテクチャ

```

harness-seed/
├── src/
│   ├── lib.rs                 # ← ReActLoop の既存コード
│   │   └── skills/            # ← NEW: MUSE 関連モジュール
│   │       ├── mod.rs          # SkillAsset, SkillManager
│   │       ├── memory.rs       # SkillMemory のロジック
│   │       ├── evaluator.rs    # Evaluation & Refinement
│   │       └── selector.rs     # Management: コンテキスト適応選択
│   │
│   └── main.rs
│
├── config/
│   ├── config.json            # ← skills セクション追加
│   └── samples/config.*.json
│
├── .triage-mail/skills/       # ← ユーザーが編集する skill assets
│   ├── email-classification/
│   │   ├── skill.prompt.md      # Creation: プロンプト定義
│   │   ├── skill.memory.md      # Memory: 実行履歴（自動更新）
│   │   ├── skill.tests.json     # Evaluation: テストケース
│   │   └── skill.versions.json  # 改善履歴（自動生成）
│   │
│   └── email-routing/
│       ├── skill.prompt.md
│       ├── skill.memory.md
│       └── ...
│
└── doc/
    └── skills-lifecycle.md    # MUSE ライフサイクル仕様

```

🏗️ Phase 1: Core Data Structures
1.1 src/skills/mod.rs - SkillAsset Definition
rustuse serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

/// 5段階ライフサイクルの単位
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillAsset {
    /// Unique identifier (e.g., "email-classification-v2")
    pub id: String,
    
    /// Creation: スキル定義
    pub definition: SkillDefinition,
    
    /// Memory: 実行メトリクス・経験
    pub memory: SkillMemory,
    
    /// Metadata: 所有権・バージョン・関連情報
    pub metadata: SkillMetadata,
    
    /// Evaluation: テスト・ベンチマーク
    pub evaluation: SkillEvaluation,
}

/// Stage 1: Creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDefinition {
    /// プロンプト本体
    pub prompt: String,
    
    /// 入出力スキーマ
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    
    /// パラメータ（temperature, max_tokens等）
    pub llm_params: HashMap<String, f32>,
    
    /// 説明（何をするスキルか）
    pub description: String,
    
    /// タグ（カテゴリ、言語等）
    pub tags: Vec<String>,
}

/// Stage 2: Memory
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillMemory {
    /// 実行履歴（最新100件）
    pub task_outcomes: Vec<TaskOutcome>,
    
    /// 文脈別の適応性スコア
    /// Key: "manufacturing" / "finance" / "lang:ja" / "*" (fallback)
    /// Value: 0.0..1.0 の成功率
    pub context_bindings: HashMap<String, f32>,
    
    /// 失敗パターン（再発防止用）
    pub failure_cases: Vec<FailurePattern>,
    
    /// 改善履歴（スキル進化の軌跡）
    pub refinement_history: Vec<RefinementRecord>,
    
    /// 最終更新時刻
    pub last_updated: DateTime<Utc>,
}

/// 1回の実行結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutcome {
    pub task_id: String,
    pub success: bool,
    pub llm_model: String,          // "gpt-4o" / "gemini-2.5-flash" etc
    pub metrics: ExecutionMetrics,
    pub context: TaskContext,        // 実行環境（業種・言語・ユーザー属性）
    pub timestamp: DateTime<Utc>,
    pub feedback: Option<String>,    // ユーザーまたは自動評価者による評価
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionMetrics {
    /// FID / IS など（画像生成タスク）
    pub quality_score: Option<f32>,
    
    /// タスク成功率（分類精度など）
    pub accuracy: Option<f32>,
    
    /// トークン数
    pub tokens_used: usize,
    
    /// 実行時間（秒）
    pub latency_secs: f32,
    
    /// コスト（$）
    pub cost_usd: f32,
}

/// 実行時のコンテキスト
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TaskContext {
    /// 業種（"manufacturing", "finance", "healthcare"等）
    pub industry: String,
    
    /// 言語（"ja", "en", "zh"等）
    pub language: String,
    
    /// ユーザーセグメント（"beginner", "expert", "enterprise"等）
    pub user_segment: String,
    
    /// その他のカスタムタグ
    pub tags: Vec<String>,
}

/// 失敗パターンを分析・保存
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailurePattern {
    pub pattern_id: String,
    
    /// どういう条件で失敗するか
    pub condition: String,  // e.g., "input contains [特殊文字] AND language == 'ja'"
    
    /// 発生件数
    pub occurrence_count: usize,
    
    /// 最後の発生時刻
    pub last_seen: DateTime<Utc>,
    
    /// 自動分析による根本原因（LLMが生成）
    pub root_cause_hypothesis: Option<String>,
}

/// スキルが改善された記録
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinementRecord {
    pub version: String,              // e.g., "1.0.0" → "1.1.0"
    pub timestamp: DateTime<Utc>,
    pub change_type: RefinementType,
    pub description: String,          // "Added context window expansion" など
    pub metrics_delta: MetricsDelta,   // 改善前後の差分
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RefinementType {
    PromptTuning,        // プロンプト内容の改善
    ParameterAdjustment, // temperature等の調整
    InputValidation,     // 入力前処理の追加
    OutputFormatting,    // 出力の正規化
    Deprecated,          // 廃止・削除
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetricsDelta {
    pub accuracy_delta: Option<f32>,   // e.g., +0.05 (5% 向上)
    pub latency_delta_ms: Option<i32>,
    pub cost_delta_pct: Option<f32>,
}

/// Stage 3: Management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub version: String,                    // Semantic versioning
    pub created_at: DateTime<Utc>,
    pub last_modified: DateTime<Utc>,
    pub author: String,                     // "user:dtmoyaji" / "agent:auto-refine"
    pub status: SkillStatus,
    
    /// バージョンアップ時に保持すべき属性
    pub preserve_on_upgrade: bool,
    
    /// ライセンス情報（改良#17との連携）
    pub license_level: Option<String>,      // "basic" / "pro" / "enterprise"
    pub reseller_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SkillStatus {
    Active,              // 利用可能
    Experimental,        // テスト中
    Deprecated,          // 非推奨（削除予定）
    Archived,            // 履歴参照用
}

/// Stage 4 & 5: Evaluation & Refinement
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillEvaluation {
    /// ユニットテスト
    pub unit_tests: Vec<EvaluationTest>,
    
    /// 最後の評価結果
    pub last_eval_result: Option<EvalResult>,
    
    /// 自動改善提案（LLMが生成）
    pub refinement_suggestions: Option<Vec<RefinementSuggestion>>,
    
    /// 推奨削除判定
    pub deprecation_recommended: bool,
    pub deprecation_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationTest {
    pub test_id: String,
    pub input: serde_json::Value,
    pub expected_output: serde_json::Value,
    pub context: TaskContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub passed: usize,
    pub failed: usize,
    pub timestamp: DateTime<Utc>,
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinementSuggestion {
    pub category: RefinementType,
    pub description: String,
    pub confidence_pct: f32,
}

🔄 Phase 2: Integration with ReActLoop
2.1 src/skills/selector.rs - Stage 3: Management
rustuse super::{SkillAsset, TaskContext};
use std::collections::HashMap;

/// スキル選択エンジン
pub struct SkillSelector {
    skills: HashMap<String, SkillAsset>,
}

impl SkillSelector {
    pub fn new(skills: Vec<SkillAsset>) -> Self {
        let mut map = HashMap::new();
        for skill in skills {
            map.insert(skill.id.clone(), skill);
        }
        Self { skills: map }
    }
    
    /// コンテキストに基づいて最適スキルを選択
    pub fn select_by_context(
        &self,
        task_type: &str,
        context: &TaskContext,
        top_k: usize,
    ) -> Vec<(String, f32)> {
        let mut candidates = Vec::new();
        
        for (skill_id, skill) in &self.skills {
            // タスク型にマッチするスキルのみ
            if !skill.metadata.status.is_active()
                || !skill.definition.tags.contains(&task_type.to_string())
            {
                continue;
            }
            
            // スキルレベルメモリから適応スコアを計算
            let base_score = self.compute_context_score(&skill.memory, context);
            
            // 失敗パターンのペナルティ
            let failure_penalty = self.compute_failure_penalty(&skill.memory, context);
            
            let final_score = base_score * (1.0 - failure_penalty);
            
            candidates.push((skill_id.clone(), final_score));
        }
        
        // スコアでソート、top_k を返す
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        candidates.into_iter().take(top_k).collect()
    }
    
    fn compute_context_score(&self, memory: &SkillMemory, ctx: &TaskContext) -> f32 {
        // 優先度: 完全一致 > industry > language > wildcard
        memory.context_bindings
            .get(&format!("{}/{}", ctx.industry, ctx.language))
            .or_else(|| memory.context_bindings.get(&ctx.industry))
            .or_else(|| memory.context_bindings.get(&ctx.language))
            .or_else(|| memory.context_bindings.get("*"))
            .copied()
            .unwrap_or(0.5)  // デフォルト
    }
    
    fn compute_failure_penalty(&self, memory: &SkillMemory, ctx: &TaskContext) -> f32 {
        let matching_failures = memory
            .failure_cases
            .iter()
            .filter(|f| f.matches_context(ctx))
            .collect::<Vec<_>>();
        
        if matching_failures.is_empty() {
            return 0.0;
        }
        
        // 失敗パターンの頻度とリセンシーでペナルティ算出
        let penalty = matching_failures.iter()
            .map(|f| {
                let freq = (f.occurrence_count as f32).min(10.0) / 10.0;
                let recency = if Utc::now().signed_duration_since(f.last_seen)
                    .num_days() < 7 {
                    1.0
                } else {
                    0.5
                };
                freq * recency * 0.1  // 最大 10% ペナルティ
            })
            .sum::<f32>()
            .min(0.3);
        
        penalty
    }
}
2.2 ReActLoop への統合
rust// src/lib.rs の既存コード修正

pub struct ReActLoop {
    brain: BrainMode,
    config: ReActConfig,
    
    // ← NEW: スキルマネージャー
    skill_manager: SkillAssetManager,
}

impl ReActLoop {
    pub async fn run_turn(&mut self, prompt: &str) -> Result<TurnResult> {
        // 1. スキル選択（メモリを参照）
        let context = TaskContext::infer_from_prompt(prompt);
        let selected_skills = self.skill_manager
            .select_by_context("general-reasoning", &context, 3)
            .await?;
        
        // 2. ReAct ループ本体の実行
        let mut thoughts = Vec::new();
        let mut actions = Vec::new();
        
        for step in 0..MAX_STEPS {
            // ... 既存の ReAct ロジック ...
            
            // 実行したアクションを記録
            actions.push(ActionRecord {
                step,
                skill_id: selected_skills[0].clone(),  // 使用スキル ID
                action_type: action.action_type.clone(),
                input: action.input.clone(),
                output: action.output.clone(),
            });
        }
        
        let final_answer = "...";
        
        // 3. 実行メトリクスを記録（Evaluation）
        for action in actions {
            let outcome = TaskOutcome {
                task_id: format!("turn-{}", step),
                success: true,  // 後で詳細判定
                llm_model: self.brain.model_id().to_string(),
                metrics: ExecutionMetrics {
                    accuracy: Some(0.95),
                    tokens_used: token_count,
                    latency_secs: elapsed.as_secs_f32(),
                    cost_usd: token_count as f32 * COST_PER_TOKEN,
                    ..Default::default()
                },
                context: context.clone(),
                timestamp: Utc::now(),
                feedback: None,
            };
            
            self.skill_manager
                .record_outcome(&action.skill_id, outcome)
                .await?;
        }
        
        // 4. スキル自動改善判定（Refinement）
        self.skill_manager.evaluate_and_retire().await?;
        
        Ok(TurnResult {
            answer: final_answer.to_string(),
            thoughts,
            actions,
            context: ContextMetrics {
                ..
            },
        })
    }
}

📝 Phase 3: Persistence & Versioning
3.1 src/skills/memory.rs - Skill Memory Persistence
rustuse std::path::PathBuf;
use tokio::fs;
use serde_yaml;

pub struct SkillMemoryPersistence {
    skills_dir: PathBuf,  // .triage-mail/skills/
}

impl SkillMemoryPersistence {
    pub async fn save_memory(&self, skill: &SkillAsset) -> Result<()> {
        // .triage-mail/skills/{skill_id}/skill.memory.md
        let memory_path = self.skills_dir
            .join(&skill.id)
            .join("skill.memory.md");
        
        let memory_md = self.render_memory_markdown(&skill.memory);
        fs::write(memory_path, memory_md).await?;
        
        Ok(())
    }
    
    fn render_memory_markdown(&self, memory: &SkillMemory) -> String {
        let mut md = String::new();
        
        md.push_str(&format!(
            "---\nlast_updated: {}\n---\n\n",
            memory.last_updated.to_rfc3339()
        ));
        
        // Context Bindings Table
        md.push_str("## Context Bindings\n\n");
        md.push_str("| Context | Success Rate |\n");
        md.push_str("|---------|---------------|\n");
        for (ctx, score) in &memory.context_bindings {
            md.push_str(&format!("| {} | {:.1}% |\n", ctx, score * 100.0));
        }
        md.push('\n');
        
        // Execution History (latest 20)
        md.push_str("## Recent Executions\n\n");
        md.push_str("| Timestamp | Task | Industry | Success | Accuracy | Latency |\n");
        md.push_str("|-----------|------|----------|---------|----------|----------|\n");
        
        for outcome in memory.task_outcomes.iter().rev().take(20) {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {:.1}% | {:.2}s |\n",
                outcome.timestamp.format("%Y-%m-%d %H:%M"),
                outcome.task_id,
                outcome.context.industry,
                if outcome.success { "✓" } else { "✗" },
                outcome.metrics.accuracy.unwrap_or(0.0) * 100.0,
                outcome.metrics.latency_secs,
            ));
        }
        md.push('\n');
        
        // Failure Patterns
        if !memory.failure_cases.is_empty() {
            md.push_str("## Known Failure Patterns\n\n");
            for failure in &memory.failure_cases {
                md.push_str(&format!(
                    "- **{}**: {} times (last: {})\n  - Hypothesis: {}\n",
                    failure.pattern_id,
                    failure.occurrence_count,
                    failure.last_seen.format("%Y-%m-%d"),
                    failure.root_cause_hypothesis.as_deref().unwrap_or("Unknown"),
                ));
            }
        }
        
        md
    }
    
    /// バージョンアップ後にメモリを復旧
    pub async fn restore_memory(&self, skill_id: &str) -> Result<SkillMemory> {
        let memory_path = self.skills_dir
            .join(skill_id)
            .join("skill.memory.md");
        
        if memory_path.exists() {
            let content = fs::read_to_string(memory_path).await?;
            // YAML frontmatter をパース → SkillMemory を復構
            parse_memory_from_markdown(&content)
        } else {
            Ok(SkillMemory::default())
        }
    }
}
3.2 Version Upgrade Migration
rust// config/config.json に skills セクション追加

{
  "version": "1.0.0",
  "skills": {
    "preserve_on_upgrade": true,         // ← バージョンアップ時に skill.memory.md を保持
    "auto_backup_before_refine": true,   // ← 改善前にバックアップ
    "evaluation_interval_hours": 24,     // ← 24時間ごとに自動評価
    "deprecation_threshold_accuracy": 0.5  // ← 50% 以下の精度で削除対象
  }
}

🧪 Phase 4: Evaluation & Auto-Refinement
4.1 src/skills/evaluator.rs - Stage 4 & 5
rustuse super::*;

pub struct SkillEvaluator {
    brain: BrainMode,
}

impl SkillEvaluator {
    /// 定期的に実行：スキルをテストして精度を確認
    pub async fn evaluate_skill(&self, skill: &mut SkillAsset) -> Result<EvalResult> {
        let mut passed = 0;
        let mut failed = 0;
        
        for test in &skill.evaluation.unit_tests {
            let result = self.run_test(skill, test).await?;
            if result {
                passed += 1;
            } else {
                failed += 1;
            }
        }
        
        let eval_result = EvalResult {
            passed,
            failed,
            timestamp: Utc::now(),
            details: format!("Passed: {}/{}", passed, passed + failed),
        };
        
        skill.evaluation.last_eval_result = Some(eval_result.clone());
        
        Ok(eval_result)
    }
    
    /// 自動改善提案を生成
    pub async fn suggest_refinements(&self, skill: &mut SkillAsset) -> Result<()> {
        // 失敗パターンの分析
        let failure_analysis = self.analyze_failures(&skill.memory).await?;
        
        // LLMに改善案を生成させる
        let suggestions = self.brain
            .call_for_refinement_suggestions(skill, &failure_analysis)
            .await?;
        
        skill.evaluation.refinement_suggestions = Some(suggestions);
        
        Ok(())
    }
    
    /// スキルの自動削除判定
    pub async fn check_deprecation(&self, skill: &mut SkillAsset) -> Result<()> {
        let recent_outcomes = skill.memory
            .task_outcomes
            .iter()
            .rev()
            .take(30)
            .collect::<Vec<_>>();
        
        if recent_outcomes.is_empty() {
            return Ok(());
        }
        
        let success_rate = recent_outcomes
            .iter()
            .filter(|o| o.success)
            .count() as f32
            / recent_outcomes.len() as f32;
        
        // 設定値（config.json の deprecation_threshold_accuracy）以下なら deprecated へ
        if success_rate < DEPRECATION_THRESHOLD {
            skill.metadata.status = SkillStatus::Deprecated;
            skill.evaluation.deprecation_recommended = true;
            skill.evaluation.deprecation_reason = 
                Some(format!("Low success rate: {:.1}%", success_rate * 100.0));
        }
        
        Ok(())
    }
    
    async fn run_test(&self, skill: &SkillAsset, test: &EvaluationTest) -> Result<bool> {
        let output = self.execute_skill_on_input(&skill.definition, &test.input)
            .await?;
        
        Ok(output == test.expected_output)
    }
    
    async fn analyze_failures(&self, memory: &SkillMemory) -> Result<String> {
        // 失敗パターンをサマリー化
        let summary = memory.failure_cases
            .iter()
            .take(5)
            .map(|f| format!("- {}: {}", f.pattern_id, f.condition))
            .collect::<Vec<_>>()
            .join("\n");
        
        Ok(summary)
    }
}

🚀 Phase 5: Practical Integration into Triage Mail
5.1 Email Classification Skill Example
yaml# .triage-mail/skills/email-classification/skill.prompt.md

---
skill_id: email-classification
version: 2.1.0
created_at: 2026-05-01T00:00:00Z
tags: [classification, email, triage]
description: "Classify incoming emails into priority levels"
---

# Email Classification Skill

You are an expert email triage agent. Classify the given email into one of these categories:

- **HIGH**: Requires immediate action (requests, deadlines < 48h, critical alerts)
- **MEDIUM**: Important but not urgent (status updates, reviews needed)
- **LOW**: Informational (newsletters, notifications, FYI)
- **SPAM**: Unsolicited promotional / irrelevant

## Input Format

\`\`\`json
{
  "subject": "string",
  "from": "string",
  "body": "string (first 500 chars)",
  "date": "ISO 8601"
}
\`\`\`

## Output Format

\`\`\`json
{
  "category": "HIGH | MEDIUM | LOW | SPAM",
  "confidence": 0.0..1.0,
  "reasoning": "brief explanation"
}
\`\`\`

---
json// .triage-mail/skills/email-classification/skill.tests.json

[
  {
    "test_id": "test-01-urgent-deadline",
    "input": {
      "subject": "URGENT: Project deadline extended to tomorrow",
      "from": "boss@company.com",
      "body": "We need the report by EOD tomorrow...",
      "date": "2026-06-08T09:00:00Z"
    },
    "expected_output": {
      "category": "HIGH",
      "confidence": 0.95,
      "reasoning": "Urgent deadline with action required"
    },
    "context": {
      "industry": "manufacturing",
      "language": "en",
      "user_segment": "manager"
    }
  },
  {
    "test_id": "test-02-newsletter",
    "input": {
      "subject": "Weekly digest: Industry news",
      "from": "newsletter@techsite.com",
      "body": "Here are this week's top stories...",
      "date": "2026-06-08T08:00:00Z"
    },
    "expected_output": {
      "category": "LOW",
      "confidence": 0.9,
      "reasoning": "Informational newsletter"
    },
    "context": {
      "industry": "finance",
      "language": "en",
      "user_segment": "analyst"
    }
  }
]

📊 Phase 6: Monitoring & Dashboarding
6.1 Skill Health Metrics
rustpub struct SkillHealthDashboard;

impl SkillHealthDashboard {
    pub async fn generate_report(skills: &[SkillAsset]) -> String {
        let mut report = String::new();
        
        report.push_str("# Skill Health Report\n\n");
        
        for skill in skills {
            let recent = skill.memory.task_outcomes
                .iter()
                .rev()
                .take(30)
                .collect::<Vec<_>>();
            
            let success_rate = if !recent.is_empty() {
                recent.iter().filter(|o| o.success).count() as f32 / recent.len() as f32
            } else {
                0.0
            };
            
            let avg_latency = if !recent.is_empty() {
                recent.iter().map(|o| o.metrics.latency_secs).sum::<f32>() / recent.len() as f32
            } else {
                0.0
            };
            
            let health = if success_rate > 0.9 {
                "🟢 Healthy"
            } else if success_rate > 0.7 {
                "🟡 Fair"
            } else {
                "🔴 Degraded"
            };
            
            report.push_str(&format!(
                "## {} {}\n- Success Rate: {:.1}%\n- Avg Latency: {:.2}s\n- Status: {}\n\n",
                skill.id, skill.metadata.version, success_rate * 100.0, avg_latency, health
            ));
        }
        
        report
    }
}

🔗 Integration with Existing Improvements
Mapping to改良点ログ
改良#既存問題MUSE解法#10短文テンプレートの登録・編集が手動SkillDefinition + SkillMemory で自動進化#16バージョンアップで設定消失skill.memory.md を自動保持・復旧#17-18ライセンスキー管理が未実装SkillMetadata.license_level + reseller_id#19i18n 対応で言語ごと別セットcontext_bindings で言語別スコアを管理#21代理店向けスキル販売体制なしskill.reseller_id で配布・更新を追跡
新規改良候補
markdown### 23. MUSE-Autoskill Framework for harness-seed
- **概要**: スキル（プロンプト・指示）を「消費物」から「進化する長期資産」へ
- **5段階ライフサイクル**:
  1. **Creation**: skill.prompt.md で定義・版管理
  2. **Memory**: skill.memory.md に実行履歴・文脈別精度を自動記録
  3. **Management**: SkillSelector が文脈に応じた最適スキルを自動選択
  4. **Evaluation**: 定期テスト・精度チェック、不良スキルを自動deprecated化
  5. **Refinement**: LLMが失敗パターンを分析 → 改善提案を自動生成

- **実装フェーズ**:
  - Phase 1: SkillAsset, SkillMemory 等コア構造体の設計
  - Phase 2: ReActLoop への統合、skill_manager の追加
  - Phase 3: .memory.md の永続化、バージョンアップ復旧
  - Phase 4: Evaluator & Auto-Refiner の実装
  - Phase 5: Triage Mail の Email Classification スキル化
  - Phase 6: HealthDashboard でスキル監視

- **メリット**:
  - スキル（テンプレート）が蓄積・進化し続ける
  - クロスタスク転移：ある顧客の成功パターンが別顧客へ自動伝播
  - バージョンアップ時の消失問題が根本解決
  - 代理店間でのスキル移植・配布が追跡可能
  - 低精度スキルは自動deprecated化 → ノイズ削減

- **テスト対象**:
  - Email Classification（高優先度）
  - Email Routing（中優先度）
  - 将来：Document Summarization, Code Generation等

🎬 Next Steps

Prototype Phase 1-2 (1-2週間)

SkillAsset 構造体の実装
ReActLoop への最小限の統合
テスト：email-classification スキルで proof-of-concept


Persistence & Versioning (1週間)

skill.memory.md のレンダリング・パース
バージョンアップ後の復旧メカニズム


Auto-Refinement (2週間)

SkillEvaluator の実装
LLM による改善提案生成
失敗パターン分析


Deploy to Triage Mail

Triage Mail の routing rules を SkillAsset として再実装
既存テンプレート短文を skill.prompt.md へ移行
ユーザーフィードバック ループの構築




作成日: 2026-06-08
参考資料:

ByteDance MUSE-Autoskill (arXiv:2605.27366)
harness-seed README.md / doc/
Triage Mail improvements.md
