use async_trait::async_trait;
use futures::{Stream, StreamExt as _};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use crate::error::CoreError;
use crate::events::AgentEvent;

pub type EventStream = Pin<Box<dyn Stream<Item = AgentEvent> + Send>>;

#[async_trait]
pub trait Dispatcher: Send + Sync {
    async fn publish(&self, event: AgentEvent) -> Result<(), CoreError>;
    async fn subscribe(&self) -> Result<EventStream, CoreError>;
}

pub struct LocalDispatcher {
    sender: Arc<broadcast::Sender<AgentEvent>>,
}

impl LocalDispatcher {
    pub fn new(capacity: usize) -> Arc<Self> {
        let (tx, _) = broadcast::channel(capacity);
        Arc::new(Self { sender: Arc::new(tx) })
    }
}

#[async_trait]
impl Dispatcher for LocalDispatcher {
    async fn publish(&self, event: AgentEvent) -> Result<(), CoreError> {
        let _ = self.sender.send(event);
        Ok(())
    }

    async fn subscribe(&self) -> Result<EventStream, CoreError> {
        let rx = self.sender.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|r| async move { r.ok() });
        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::AgentEvent;
    use crate::types::PromptRef;
    use futures::StreamExt as FuturesStreamExt;

    #[tokio::test]
    async fn local_dispatcher_publish_then_receive() {
        let d = LocalDispatcher::new(64);
        let mut stream = d.subscribe().await.unwrap();

        d.publish(AgentEvent::ModelRequest {
            provider: "claude".to_string(),
            prompt_ref: PromptRef::new(),
            token_estimate: 42,
        }).await.unwrap();

        let received = FuturesStreamExt::next(&mut stream).await.unwrap();
        assert!(matches!(received, AgentEvent::ModelRequest { token_estimate: 42, .. }));
    }

    #[tokio::test]
    async fn dispatcher_delivers_to_multiple_subscribers() {
        let d = LocalDispatcher::new(64);
        let mut s1 = d.subscribe().await.unwrap();
        let mut s2 = d.subscribe().await.unwrap();

        d.publish(AgentEvent::ModelRequest {
            provider: "ollama".into(),
            prompt_ref: PromptRef::new(),
            token_estimate: 7,
        }).await.unwrap();

        let e1 = FuturesStreamExt::next(&mut s1).await.unwrap();
        let e2 = FuturesStreamExt::next(&mut s2).await.unwrap();
        assert!(matches!(e1, AgentEvent::ModelRequest { token_estimate: 7, .. }));
        assert!(matches!(e2, AgentEvent::ModelRequest { token_estimate: 7, .. }));
    }
}
