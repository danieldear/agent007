use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CompactLevel {
    Full,
    Compact,
    Aggressive,
}

impl CompactLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Compact => "compact",
            Self::Aggressive => "aggressive",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudget {
    pub max_prompt_tokens: u64,
    pub reserve_tokens: u64,
    pub max_response_tokens: u64,
}

impl Default for TokenBudget {
    fn default() -> Self {
        Self {
            max_prompt_tokens: 8_000,
            reserve_tokens: 1_500,
            max_response_tokens: 2_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetEstimate {
    pub max_prompt_tokens: u64,
    pub reserve_tokens: u64,
    pub max_response_tokens: u64,
    pub usable_prompt_tokens: u64,
    pub estimated_prompt_tokens: u64,
    pub remaining_prompt_tokens: i64,
    pub estimated_total_tokens: u64,
    pub recommended_level: CompactLevel,
    pub should_compile_context: bool,
    pub should_use_artifacts: bool,
    pub notes: Vec<String>,
}

pub fn estimate_tokens(text: &str) -> u64 {
    ((text.chars().count() as f64) / 4.0).ceil() as u64
}

impl TokenBudget {
    pub fn usable_prompt_tokens(&self) -> u64 {
        self.max_prompt_tokens.saturating_sub(self.reserve_tokens)
    }

    pub fn estimate_prompt(&self, estimated_prompt_tokens: u64) -> BudgetEstimate {
        let usable_prompt_tokens = self.usable_prompt_tokens();
        let remaining_prompt_tokens = usable_prompt_tokens as i64 - estimated_prompt_tokens as i64;
        let recommended_level = if estimated_prompt_tokens <= usable_prompt_tokens / 2 {
            CompactLevel::Full
        } else if estimated_prompt_tokens <= usable_prompt_tokens {
            CompactLevel::Compact
        } else {
            CompactLevel::Aggressive
        };

        let mut notes = Vec::new();
        if estimated_prompt_tokens > usable_prompt_tokens {
            notes.push(format!(
                "Prompt estimate {} exceeds usable budget {}. Switch to aggressive context and artifact references.",
                estimated_prompt_tokens, usable_prompt_tokens
            ));
        } else if estimated_prompt_tokens > usable_prompt_tokens * 3 / 4 {
            notes.push(format!(
                "Prompt estimate {} is close to the usable budget {}. Prefer compact summaries.",
                estimated_prompt_tokens, usable_prompt_tokens
            ));
        } else {
            notes.push("Prompt size is comfortably within budget.".to_string());
        }

        let should_compile_context = estimated_prompt_tokens > usable_prompt_tokens / 4;
        let should_use_artifacts = estimated_prompt_tokens > usable_prompt_tokens;

        BudgetEstimate {
            max_prompt_tokens: self.max_prompt_tokens,
            reserve_tokens: self.reserve_tokens,
            max_response_tokens: self.max_response_tokens,
            usable_prompt_tokens,
            estimated_prompt_tokens,
            remaining_prompt_tokens,
            estimated_total_tokens: estimated_prompt_tokens + self.max_response_tokens,
            recommended_level,
            should_compile_context,
            should_use_artifacts,
            notes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_is_rough_char_ratio() {
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    #[test]
    fn budget_prefers_full_when_prompt_is_small() {
        let budget = TokenBudget::default();
        let report = budget.estimate_prompt(1_000);
        assert_eq!(report.recommended_level, CompactLevel::Full);
        assert!(report.remaining_prompt_tokens > 0);
    }

    #[test]
    fn budget_prefers_aggressive_when_prompt_exceeds_usable_budget() {
        let budget = TokenBudget::default();
        let report = budget.estimate_prompt(10_000);
        assert_eq!(report.recommended_level, CompactLevel::Aggressive);
        assert!(report.should_use_artifacts);
    }
}
