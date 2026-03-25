use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use agent007_core::types::{AgentId, PromptRef};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackEntry {
    pub id: Uuid,
    pub agent_id: AgentId,
    pub prompt_ref: PromptRef,
    pub skill_name: Option<String>,
    pub model: String,
    pub outcome: Outcome,
    pub reward: Option<f32>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Outcome {
    Success,
    Failure { reason: String },
    UserRating { score: f32 },
    ToolError { tool: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LearningEvent {
    PromptImproved { skill_name: String, old_reward: f32, new_reward: f32 },
    FeedbackRecorded { agent_id: AgentId, reward: f32 },
    OptimizerTriggered { skill_name: String },
}

impl LearningEvent {
    /// Extract the reward value for telemetry/WebSocket streaming.
    pub fn avg_reward(&self) -> f64 {
        match self {
            LearningEvent::FeedbackRecorded { reward, .. } => *reward as f64,
            LearningEvent::PromptImproved { new_reward, .. } => *new_reward as f64,
            LearningEvent::OptimizerTriggered { .. } => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_entry_roundtrips_json() {
        let entry = FeedbackEntry {
            id: Uuid::new_v4(),
            agent_id: AgentId::new(),
            prompt_ref: PromptRef::new(),
            skill_name: Some("code-review".to_string()),
            model: "claude".to_string(),
            outcome: Outcome::Success,
            reward: Some(0.8),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: FeedbackEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, entry.id);
        assert_eq!(back.model, "claude");
        assert_eq!(back.reward, Some(0.8));
    }

    #[test]
    fn outcome_failure_serializes_with_reason() {
        let o = Outcome::Failure { reason: "timeout".to_string() };
        let json = serde_json::to_string(&o).unwrap();
        let back: Outcome = serde_json::from_str(&json).unwrap();
        if let Outcome::Failure { reason } = back {
            assert_eq!(reason, "timeout");
        } else {
            panic!("expected Failure");
        }
    }

    #[test]
    fn learning_event_prompt_improved_has_rewards() {
        let e = LearningEvent::PromptImproved {
            skill_name: "review-pr".to_string(),
            old_reward: 0.2,
            new_reward: 0.7,
        };
        if let LearningEvent::PromptImproved { old_reward, new_reward, .. } = e {
            assert!((old_reward - 0.2).abs() < 0.001);
            assert!((new_reward - 0.7).abs() < 0.001);
        }
    }
}
