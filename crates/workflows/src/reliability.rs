use serde::{Deserialize, Serialize};

use crate::types::{BudgetConfig, BudgetUsed, ReliabilityConfig, WorkflowDef};

const DEFAULT_GUARDRAIL_TERMS: &[&str] = &[
    "rm -rf",
    "drop table",
    "truncate table",
    "delete from",
    "format disk",
    "wipe database",
];

const DEFAULT_LOW_CONFIDENCE_TERMS: &[&str] = &[
    "confidence: low",
    "\"confidence\":\"low\"",
    "confidence=low",
    "low confidence",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReliabilityTransitionKind {
    Continue,
    Retry,
    Degrade,
    EscalateApproval,
    Abort,
    GuardrailBlocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReliabilityTransition {
    pub step_id: String,
    pub kind: ReliabilityTransitionKind,
    pub reason_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ReliabilityTransition {
    pub fn new(
        step_id: impl Into<String>,
        kind: ReliabilityTransitionKind,
        reason_code: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            kind,
            reason_code: reason_code.into(),
            detail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardrailDecision {
    Allow {
        reason_code: String,
    },
    Block {
        reason_code: String,
        category: String,
        rationale: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscalationDecision {
    None,
    RequestApproval { reason_code: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetDecision {
    Continue {
        reason_code: String,
    },
    Degrade {
        reason_code: String,
        target_chars: usize,
    },
    Abort {
        reason_code: String,
    },
}

#[derive(Debug, Clone)]
pub struct ReliabilityPolicy {
    pub recovery_enabled: bool,
    pub budget_governor_enabled: bool,
    pub guardrails_enabled: bool,
    pub confidence_escalation_enabled: bool,
    pub max_step_retries: u32,
    pub max_degradations_per_run: u32,
    pub degrade_output_chars: usize,
    pub guardrail_terms: Vec<String>,
    pub confidence_low_terms: Vec<String>,
    pub confidence_missing_requires_approval: bool,
}

impl ReliabilityPolicy {
    pub fn from_env() -> Self {
        let all_enabled = env_bool("AGENT007_RELIABILITY_ENABLED", false);
        Self {
            recovery_enabled: env_bool("AGENT007_RELIABILITY_RECOVERY", all_enabled),
            budget_governor_enabled: env_bool("AGENT007_RELIABILITY_BUDGET_GOVERNOR", all_enabled),
            guardrails_enabled: env_bool("AGENT007_RELIABILITY_GUARDRAILS", all_enabled),
            confidence_escalation_enabled: env_bool(
                "AGENT007_RELIABILITY_CONFIDENCE_ESCALATION",
                all_enabled,
            ),
            max_step_retries: env_u32("AGENT007_RELIABILITY_MAX_STEP_RETRIES", 2),
            max_degradations_per_run: env_u32("AGENT007_RELIABILITY_MAX_DEGRADATIONS", 1),
            degrade_output_chars: env_usize("AGENT007_RELIABILITY_DEGRADE_OUTPUT_CHARS", 400),
            guardrail_terms: env_list(
                "AGENT007_RELIABILITY_GUARDRAIL_TERMS",
                DEFAULT_GUARDRAIL_TERMS,
            ),
            confidence_low_terms: env_list(
                "AGENT007_RELIABILITY_LOW_CONFIDENCE_TERMS",
                DEFAULT_LOW_CONFIDENCE_TERMS,
            ),
            confidence_missing_requires_approval: env_bool(
                "AGENT007_RELIABILITY_CONFIDENCE_REQUIRE_ON_MISSING",
                false,
            ),
        }
    }

    pub fn from_workflow(def: &WorkflowDef) -> Self {
        let mut policy = Self::from_env();
        if let Some(config) = &def.reliability {
            policy.apply_workflow_overrides(config);
        }
        policy
    }

    fn apply_workflow_overrides(&mut self, config: &ReliabilityConfig) {
        if let Some(enabled) = config.enabled {
            self.recovery_enabled = enabled;
            self.budget_governor_enabled = enabled;
            self.guardrails_enabled = enabled;
            self.confidence_escalation_enabled = enabled;
        }

        if let Some(recovery) = &config.recovery {
            if let Some(enabled) = recovery.enabled {
                self.recovery_enabled = enabled;
            }
            if let Some(max_step_retries) = recovery.max_step_retries {
                self.max_step_retries = max_step_retries;
            }
        }

        if let Some(budget_governor) = &config.budget_governor {
            if let Some(enabled) = budget_governor.enabled {
                self.budget_governor_enabled = enabled;
            }
            if let Some(max_degradations_per_run) = budget_governor.max_degradations_per_run {
                self.max_degradations_per_run = max_degradations_per_run;
            }
            if let Some(degrade_output_chars) = budget_governor.degrade_output_chars {
                self.degrade_output_chars = degrade_output_chars;
            }
        }

        if let Some(guardrails) = &config.guardrails {
            if let Some(enabled) = guardrails.enabled {
                self.guardrails_enabled = enabled;
            }
            if let Some(terms) = &guardrails.terms {
                let filtered = terms
                    .iter()
                    .map(|term| term.trim())
                    .filter(|term| !term.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                if !filtered.is_empty() {
                    self.guardrail_terms = filtered;
                }
            }
        }

        if let Some(confidence) = &config.confidence {
            if let Some(enabled) = confidence.enabled {
                self.confidence_escalation_enabled = enabled;
            }
            if let Some(low_terms) = &confidence.low_terms {
                let filtered = low_terms
                    .iter()
                    .map(|term| term.trim())
                    .filter(|term| !term.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                if !filtered.is_empty() {
                    self.confidence_low_terms = filtered;
                }
            }
            if let Some(missing_requires_approval) = confidence.missing_requires_approval {
                self.confidence_missing_requires_approval = missing_requires_approval;
            }
        }
    }
}

pub fn evaluate_guardrail(
    step_id: &str,
    rendered_prompt: &str,
    policy: &ReliabilityPolicy,
) -> GuardrailDecision {
    if !policy.guardrails_enabled {
        return GuardrailDecision::Allow {
            reason_code: "guardrails-disabled".to_string(),
        };
    }

    let haystack = format!("{} {}", step_id, rendered_prompt);
    let haystack_spaced = normalize_with_spaces(&haystack);
    let haystack_compact = normalize_compact(&haystack);
    for term in &policy.guardrail_terms {
        let term = term.trim();
        if term.is_empty() {
            continue;
        }
        let needle_spaced = normalize_with_spaces(term);
        let needle_compact = normalize_compact(term);
        if (!needle_spaced.is_empty() && haystack_spaced.contains(&needle_spaced))
            || (!needle_compact.is_empty() && haystack_compact.contains(&needle_compact))
        {
            return GuardrailDecision::Block {
                reason_code: "risky-operation-detected".to_string(),
                category: "destructive-operation".to_string(),
                rationale: format!("matched guarded term '{term}'"),
            };
        }
    }

    GuardrailDecision::Allow {
        reason_code: "guardrail-pass".to_string(),
    }
}

pub fn evaluate_confidence(output: &str, policy: &ReliabilityPolicy) -> EscalationDecision {
    if !policy.confidence_escalation_enabled {
        return EscalationDecision::None;
    }

    let normalized_spaced = normalize_with_spaces(output);
    let normalized_compact = normalize_compact(output);
    if policy.confidence_low_terms.iter().any(|term| {
        let spaced = normalize_with_spaces(term);
        let compact = normalize_compact(term);
        (!spaced.is_empty() && normalized_spaced.contains(&spaced))
            || (!compact.is_empty() && normalized_compact.contains(&compact))
    }) {
        return EscalationDecision::RequestApproval {
            reason_code: "low-confidence-detected".to_string(),
        };
    }

    if policy.confidence_missing_requires_approval && !normalized_spaced.contains("confidence") {
        return EscalationDecision::RequestApproval {
            reason_code: "confidence-missing".to_string(),
        };
    }

    EscalationDecision::None
}

pub fn evaluate_budget_decision(
    budget: &BudgetConfig,
    used: &BudgetUsed,
    additional_tokens: u64,
    additional_usd: f64,
    degradation_count: u32,
    policy: &ReliabilityPolicy,
) -> BudgetDecision {
    let projected = BudgetUsed {
        tokens: used.tokens.saturating_add(additional_tokens),
        estimated_usd: used.estimated_usd + additional_usd,
    };
    if !is_budget_exceeded(budget, &projected) {
        return BudgetDecision::Continue {
            reason_code: "budget-ok".to_string(),
        };
    }

    if policy.budget_governor_enabled && degradation_count < policy.max_degradations_per_run {
        return BudgetDecision::Degrade {
            reason_code: "budget-exceeded-degrade".to_string(),
            target_chars: policy.degrade_output_chars,
        };
    }

    BudgetDecision::Abort {
        reason_code: "budget-exceeded-abort".to_string(),
    }
}

pub fn apply_degradation(content: &str, target_chars: usize) -> String {
    let mut chars = content.chars();
    let preview: String = chars.by_ref().take(target_chars).collect();
    if chars.next().is_none() {
        return content.to_string();
    }
    format!("{preview}\n[degraded]")
}

fn is_budget_exceeded(budget: &BudgetConfig, used: &BudgetUsed) -> bool {
    let token_exceeded = budget
        .max_tokens_per_session
        .map(|limit| used.tokens > limit)
        .unwrap_or(false);
    let usd_exceeded = budget
        .max_usd_per_task
        .map(|limit| used.estimated_usd > limit)
        .unwrap_or(false);
    token_exceeded || usd_exceeded
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .map(|value| {
            matches!(
                value.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_list(key: &str, defaults: &[&str]) -> Vec<String> {
    std::env::var(key)
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| defaults.iter().map(|item| item.to_string()).collect())
}

fn normalize_with_spaces(input: &str) -> String {
    let mut normalized = String::with_capacity(input.len());
    let mut last_was_space = true;
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            normalized.push(c.to_ascii_lowercase());
            last_was_space = false;
        } else if !last_was_space {
            normalized.push(' ');
            last_was_space = true;
        }
    }
    normalized.trim().to_string()
}

fn normalize_compact(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
pub fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ReliabilityPolicy {
        ReliabilityPolicy {
            recovery_enabled: true,
            budget_governor_enabled: true,
            guardrails_enabled: true,
            confidence_escalation_enabled: true,
            max_step_retries: 2,
            max_degradations_per_run: 1,
            degrade_output_chars: 10,
            guardrail_terms: vec!["drop table".to_string()],
            confidence_low_terms: vec!["confidence: low".to_string()],
            confidence_missing_requires_approval: false,
        }
    }

    #[test]
    fn guardrail_blocks_risky_prompt() {
        let decision = evaluate_guardrail("step", "please drop table users", &policy());
        assert!(matches!(decision, GuardrailDecision::Block { .. }));
    }

    #[test]
    fn confidence_escalates_on_low_signal() {
        let decision = evaluate_confidence("result\nconfidence: low", &policy());
        assert!(matches!(
            decision,
            EscalationDecision::RequestApproval { .. }
        ));
    }

    #[test]
    fn guardrail_blocks_obfuscated_prompt() {
        let decision = evaluate_guardrail("step", "please dr-op ta_ble users", &policy());
        assert!(matches!(decision, GuardrailDecision::Block { .. }));
    }

    #[test]
    fn budget_decision_degrades_before_abort() {
        let decision = evaluate_budget_decision(
            &BudgetConfig {
                max_tokens_per_session: Some(5),
                max_usd_per_task: None,
                alert_at_percent: None,
                on_exceed: Some("stop".to_string()),
            },
            &BudgetUsed {
                tokens: 4,
                estimated_usd: 0.0,
            },
            4,
            0.0,
            0,
            &policy(),
        );
        assert!(matches!(decision, BudgetDecision::Degrade { .. }));
    }

    #[test]
    fn degradation_truncates_output() {
        let degraded = apply_degradation("abcdefghijklmnopqrstuvwxyz", 8);
        assert!(degraded.starts_with("abcdefgh"));
        assert!(degraded.contains("[degraded]"));
    }
}
