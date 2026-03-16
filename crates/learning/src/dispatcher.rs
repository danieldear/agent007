use futures::StreamExt as _;
use std::pin::Pin;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

pub struct LearningDispatcher {
    sender: broadcast::Sender<crate::types::LearningEvent>,
}

impl LearningDispatcher {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { sender: tx }
    }

    pub fn publish(&self, event: crate::types::LearningEvent) -> Result<(), crate::error::LearningError> {
        let _ = self.sender.send(event);
        Ok(())
    }

    pub fn subscribe(&self) -> Pin<Box<dyn futures::Stream<Item = crate::types::LearningEvent> + Send>> {
        let rx = self.sender.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|r| async move { r.ok() });
        Box::pin(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::LearningEvent;
    use futures::StreamExt as FuturesStreamExt;

    #[tokio::test]
    async fn new_creates_working_pub_sub_channel() {
        let d = LearningDispatcher::new(64);
        let mut stream = d.subscribe();

        d.publish(LearningEvent::OptimizerTriggered {
            skill_name: "test-skill".to_string(),
        })
        .unwrap();

        let received = FuturesStreamExt::next(&mut stream).await.unwrap();
        assert!(matches!(received, LearningEvent::OptimizerTriggered { .. }));
    }

    #[tokio::test]
    async fn publish_feedback_recorded_is_received_by_subscriber() {
        let d = LearningDispatcher::new(64);
        let mut stream = d.subscribe();

        d.publish(LearningEvent::FeedbackRecorded {
            agent_id: agent007_core::types::AgentId::new(),
            reward: 0.9,
        })
        .unwrap();

        let received = FuturesStreamExt::next(&mut stream).await.unwrap();
        if let LearningEvent::FeedbackRecorded { reward, .. } = received {
            assert!((reward - 0.9).abs() < 0.001);
        } else {
            panic!("expected FeedbackRecorded");
        }
    }

    #[tokio::test]
    async fn publish_optimizer_triggered_received_by_unrelated_subscribers() {
        let d = LearningDispatcher::new(64);
        let mut s1 = d.subscribe();
        let mut s2 = d.subscribe();

        d.publish(LearningEvent::OptimizerTriggered {
            skill_name: "code-review".to_string(),
        })
        .unwrap();

        let e1 = FuturesStreamExt::next(&mut s1).await.unwrap();
        let e2 = FuturesStreamExt::next(&mut s2).await.unwrap();

        assert!(matches!(e1, LearningEvent::OptimizerTriggered { .. }));
        assert!(matches!(e2, LearningEvent::OptimizerTriggered { .. }));
    }

    #[tokio::test]
    async fn subscriber_receives_events_in_order() {
        let d = LearningDispatcher::new(64);
        let mut stream = d.subscribe();

        let agent_id = agent007_core::types::AgentId::new();

        d.publish(LearningEvent::FeedbackRecorded {
            agent_id: agent_id.clone(),
            reward: 0.1,
        })
        .unwrap();
        d.publish(LearningEvent::OptimizerTriggered {
            skill_name: "skill-a".to_string(),
        })
        .unwrap();
        d.publish(LearningEvent::PromptImproved {
            skill_name: "skill-a".to_string(),
            old_reward: 0.1,
            new_reward: 0.8,
        })
        .unwrap();

        let first = FuturesStreamExt::next(&mut stream).await.unwrap();
        let second = FuturesStreamExt::next(&mut stream).await.unwrap();
        let third = FuturesStreamExt::next(&mut stream).await.unwrap();

        assert!(matches!(first, LearningEvent::FeedbackRecorded { .. }));
        assert!(matches!(second, LearningEvent::OptimizerTriggered { .. }));
        assert!(matches!(third, LearningEvent::PromptImproved { .. }));
    }
}
