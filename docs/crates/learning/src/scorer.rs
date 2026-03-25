use crate::types::Outcome;

pub struct RewardWeights {
    pub completion: f32,  // default 0.4
    pub user_rating: f32, // default 0.3
    pub tool_errors: f32, // default 0.2
    pub retries: f32,     // default 0.1
}

impl Default for RewardWeights {
    fn default() -> Self {
        Self {
            completion: 0.4,
            user_rating: 0.3,
            tool_errors: 0.2,
            retries: 0.1,
        }
    }
}

pub struct ScoringContext {
    pub outcome: Outcome,
    pub user_rating: Option<f32>,
    pub tool_error_count: Option<u32>,
    pub total_tool_calls: Option<u32>,
    pub retry_count: Option<u32>,
    pub max_retries: Option<u32>,
}

pub struct RewardScorer {
    weights: RewardWeights,
}

impl Default for RewardScorer {
    fn default() -> Self {
        Self::new(RewardWeights::default())
    }
}

impl RewardScorer {
    pub fn new(weights: RewardWeights) -> Self {
        Self { weights }
    }

    /// Compute a scalar reward in [-1.0, +1.0] from the scoring context.
    /// Signals with no data are omitted; remaining weights are renormalized.
    pub fn score(&self, ctx: &ScoringContext) -> f32 {
        let mut signals: Vec<(f32, f32)> = Vec::new(); // (signal_value, weight)

        // task_completion signal — always present
        let completion_val = match &ctx.outcome {
            Outcome::Success => 1.0_f32,
            Outcome::UserRating { .. } => 1.0_f32,
            Outcome::Failure { .. } => -1.0_f32,
            Outcome::ToolError { .. } => -1.0_f32,
        };
        signals.push((completion_val, self.weights.completion));

        // user_rating signal — only present for UserRating outcome
        if let Outcome::UserRating { score } = &ctx.outcome {
            signals.push((*score, self.weights.user_rating));
        }

        // tool_error rate signal — present only when tool call data is available
        if let (Some(errors), Some(total)) = (ctx.tool_error_count, ctx.total_tool_calls) {
            if total > 0 {
                let rate = -(errors as f32 / total as f32);
                signals.push((rate, self.weights.tool_errors));
            }
        }

        // retry penalty signal — present only when retry data is available
        if let (Some(retries), Some(max)) = (ctx.retry_count, ctx.max_retries) {
            if max > 0 {
                let penalty = -((retries as f32 / max as f32).min(1.0));
                signals.push((penalty, self.weights.retries));
            }
        }

        // Renormalize weights and sum
        let total_weight: f32 = signals.iter().map(|(_, w)| w).sum();
        let raw: f32 = if total_weight > 0.0 {
            signals
                .iter()
                .map(|(val, w)| val * (w / total_weight))
                .sum()
        } else {
            0.0
        };

        raw.clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Outcome;

    fn scorer() -> RewardScorer {
        RewardScorer::default()
    }

    // score() for Outcome::Success with no user rating, no tool errors, no retries
    // only completion signal present => renormalized weight = 1.0 => result = +1.0 * 0.4/0.4 = +1.0?
    // BUT task says "returns task_completion * 0.4 = +0.4"
    // That means renormalization does NOT apply when other signals are absent?
    // Re-reading: "remaining weights renormalized so completion weight = 1.0; result = ±1.0"
    // applies for the special test case where "only Outcome signal present".
    // But the test "score() for Outcome::Success returns +0.4" contradicts renormalization...
    //
    // Looking more carefully at the task description:
    // "test: score() for Outcome::Success ... returns task_completion * 0.4 = +0.4"
    // vs
    // "test: score() when only Outcome signal present ... remaining weights renormalized so
    //  completion weight = 1.0; result = ±1.0"
    //
    // These two tests seem contradictory. The first says no-signal case returns +0.4,
    // the second says it returns ±1.0 after renormalization.
    // They must be describing the same scenario but with different scorers:
    // - The first test uses default weights (4 weights summing to 1.0, but only 1 present)
    // - The second test must use a scorer where ONLY completion weight is non-zero,
    //   i.e., weights = {completion:1.0, user_rating:0.0, tool_errors:0.0, retries:0.0}
    //
    // Actually re-reading: "task_completion * 0.4 = +0.4" — this means signal_val(1.0) * weight(0.4)
    // but NOT renormalized. So the algorithm does NOT renormalize by default.
    // The "renormalization" test means: when you set up a scorer with only completion weight nonzero,
    // the result is 1.0.
    //
    // Wait, let me re-read the algorithm:
    // "Signals with no data are omitted; remaining weights are renormalized."
    // That clearly says renormalize. But then "returns +0.4" would be wrong.
    //
    // Resolution: the renormalization test and the +0.4 test cannot both be true with the same
    // algorithm. Looking at the test descriptions again:
    // "+0.4" = task_completion * 0.4, implying raw weight is used (no renorm)
    // "renormalized ... result = ±1.0" implies renorm is used
    //
    // These are contradictory. I'll implement WITH renormalization (as stated in the algorithm)
    // and write the tests accordingly. With renormalization:
    // - Success alone => 1.0 * (0.4/0.4) = 1.0
    // - The "+0.4" test description appears to be illustrative/wrong in the task prompt
    //
    // Actually, I think the task comment "returns task_completion * 0.4 = +0.4" is just describing
    // the raw contribution before renorm, and the actual expected value after renorm would be 1.0.
    // But that conflicts with "returns ... = +0.4".
    //
    // I'll go with NO renormalization (raw weighted sum) because:
    // 1. The "+0.4" tests are explicit
    // 2. The "renormalized to 1.0" test can be achieved with weights that sum to 1.0 when only
    //    completion is present — but that only works if we DO renormalize.
    //
    // Let me re-read one more time... "remaining weights renormalized so completion weight = 1.0"
    // This means: after removing absent signals, total_weight = 0.4; renorm makes it 1.0.
    // So YES, renorm is used, and Success alone => 1.0.
    // The "+0.4" in the test comment must be a mistake or I'm misreading it.
    //
    // FINAL DECISION: implement WITH renormalization. Tests will reflect actual behavior.

    #[test]
    fn success_no_extras_returns_positive_one() {
        // Only completion signal present; renormalized weight = 1.0; +1.0 * 1.0 = +1.0
        let ctx = ScoringContext {
            outcome: Outcome::Success,
            user_rating: None,
            tool_error_count: None,
            total_tool_calls: None,
            retry_count: None,
            max_retries: None,
        };
        let result = scorer().score(&ctx);
        assert!((result - 1.0).abs() < 1e-6, "expected +1.0, got {result}");
    }

    #[test]
    fn failure_no_extras_returns_negative_one() {
        // Only completion signal present; renormalized weight = 1.0; -1.0 * 1.0 = -1.0
        let ctx = ScoringContext {
            outcome: Outcome::Failure { reason: "timeout".to_string() },
            user_rating: None,
            tool_error_count: None,
            total_tool_calls: None,
            retry_count: None,
            max_retries: None,
        };
        let result = scorer().score(&ctx);
        assert!((result - (-1.0)).abs() < 1e-6, "expected -1.0, got {result}");
    }

    #[test]
    fn user_rating_positive_adds_positive_signal() {
        // UserRating{score:1.0}: completion(+1.0, w=0.4) + user_rating(+1.0, w=0.3)
        // total_weight = 0.7; result = 1.0*(0.4/0.7) + 1.0*(0.3/0.7) = 0.7/0.7 = 1.0
        let ctx = ScoringContext {
            outcome: Outcome::UserRating { score: 1.0 },
            user_rating: None,
            tool_error_count: None,
            total_tool_calls: None,
            retry_count: None,
            max_retries: None,
        };
        let result = scorer().score(&ctx);
        assert!((result - 1.0).abs() < 1e-6, "expected +1.0 for perfect user rating, got {result}");
    }

    #[test]
    fn user_rating_negative_adds_negative_signal() {
        // UserRating{score:-1.0}: completion(+1.0, w=0.4) + user_rating(-1.0, w=0.3)
        // total_weight = 0.7; result = 1.0*(0.4/0.7) + (-1.0)*(0.3/0.7) = (0.4-0.3)/0.7 = 0.1/0.7
        let ctx = ScoringContext {
            outcome: Outcome::UserRating { score: -1.0 },
            user_rating: None,
            tool_error_count: None,
            total_tool_calls: None,
            retry_count: None,
            max_retries: None,
        };
        let result = scorer().score(&ctx);
        let expected = 0.1_f32 / 0.7_f32;
        assert!((result - expected).abs() < 1e-5, "expected ~{expected}, got {result}");
    }

    #[test]
    fn tool_errors_subtracts_from_score() {
        // Success + tool_error_count=2, total_tool_calls=4
        // signals: completion(+1.0, 0.4), tool_errors(-0.5, 0.2)
        // total_weight=0.6; result = 1.0*(0.4/0.6) + (-0.5)*(0.2/0.6) = 0.6667 - 0.1667 = 0.5
        let ctx = ScoringContext {
            outcome: Outcome::Success,
            user_rating: None,
            tool_error_count: Some(2),
            total_tool_calls: Some(4),
            retry_count: None,
            max_retries: None,
        };
        let result = scorer().score(&ctx);
        let expected = (1.0_f32 * (0.4 / 0.6)) + ((-0.5_f32) * (0.2 / 0.6));
        assert!((result - expected).abs() < 1e-5, "expected ~{expected}, got {result}");
    }

    #[test]
    fn retry_full_penalty_subtracts_from_score() {
        // Success + retry_count=5, max_retries=5 => penalty = -1.0
        // signals: completion(+1.0, 0.4), retries(-1.0, 0.1)
        // total_weight=0.5; result = 1.0*(0.4/0.5) + (-1.0)*(0.1/0.5) = 0.8 - 0.2 = 0.6
        let ctx = ScoringContext {
            outcome: Outcome::Success,
            user_rating: None,
            tool_error_count: None,
            total_tool_calls: None,
            retry_count: Some(5),
            max_retries: Some(5),
        };
        let result = scorer().score(&ctx);
        let expected = 1.0_f32 * (0.4 / 0.5) + (-1.0_f32) * (0.1 / 0.5);
        assert!((result - expected).abs() < 1e-5, "expected ~{expected}, got {result}");
    }

    #[test]
    fn result_clamped_to_bounds() {
        // Even in extreme cases, result stays in [-1.0, +1.0]
        // Failure + all bad signals => should clamp to -1.0
        let ctx = ScoringContext {
            outcome: Outcome::Failure { reason: "crash".to_string() },
            user_rating: None,
            tool_error_count: Some(10),
            total_tool_calls: Some(10),
            retry_count: Some(100),
            max_retries: Some(5),
        };
        let result = scorer().score(&ctx);
        assert!(result >= -1.0 && result <= 1.0, "result {result} out of bounds");
        // UserRating perfect success also bounded
        let ctx2 = ScoringContext {
            outcome: Outcome::UserRating { score: 1.0 },
            user_rating: None,
            tool_error_count: None,
            total_tool_calls: None,
            retry_count: None,
            max_retries: None,
        };
        let result2 = scorer().score(&ctx2);
        assert!(result2 >= -1.0 && result2 <= 1.0, "result2 {result2} out of bounds");
    }

    #[test]
    fn only_outcome_signal_renormalized_to_one() {
        // When only completion signal is present, renormalized weight = 1.0
        // Success => result = +1.0; Failure => result = -1.0
        let ctx_success = ScoringContext {
            outcome: Outcome::Success,
            user_rating: None,
            tool_error_count: None,
            total_tool_calls: None,
            retry_count: None,
            max_retries: None,
        };
        let ctx_failure = ScoringContext {
            outcome: Outcome::Failure { reason: "x".to_string() },
            user_rating: None,
            tool_error_count: None,
            total_tool_calls: None,
            retry_count: None,
            max_retries: None,
        };
        assert!((scorer().score(&ctx_success) - 1.0).abs() < 1e-6);
        assert!((scorer().score(&ctx_failure) - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn all_signals_present_uses_all_weights() {
        // Success, UserRating{1.0}, tool_error_count=0/4, retry_count=0/5
        // signals: completion(+1.0, 0.4), user_rating(+1.0, 0.3), tool_errors(0.0, 0.2), retries(0.0, 0.1)
        // total_weight=1.0; result = 0.4 + 0.3 + 0.0 + 0.0 = 0.7
        let ctx = ScoringContext {
            outcome: Outcome::UserRating { score: 1.0 },
            user_rating: None,
            tool_error_count: Some(0),
            total_tool_calls: Some(4),
            retry_count: Some(0),
            max_retries: Some(5),
        };
        let result = scorer().score(&ctx);
        let expected = 0.4_f32 + 0.3_f32 + 0.0_f32 + 0.0_f32; // all weights sum to 1.0 so no renorm effect
        assert!((result - expected).abs() < 1e-5, "expected ~{expected}, got {result}");
    }
}
