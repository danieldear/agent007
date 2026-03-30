use axum::{
    extract::{State, WebSocketUpgrade},
    response::IntoResponse,
};
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;

use agent007_core::paths::agent007_home;
use agent007_core::dispatcher::Dispatcher;
use crate::server::AppState;
use crate::metrics::DashboardMetrics;

/// Messages broadcast to WebSocket clients.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    AgentEvent {
        source: String,
        payload: Value,
    },
    LearningEvent {
        avg_reward: f64,
        payload: Value,
    },
    StatusUpdate {
        metrics: DashboardMetrics,
    },
}

/// axum handler — upgrades the HTTP connection to a WebSocket, then streams
/// `AgentEvent` and `LearningEvent` to the browser.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    let mut agent_stream: agent007_core::dispatcher::EventStream =
        match state.dispatcher.subscribe().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("failed to subscribe to agent events: {e}");
                return;
            }
        };
    let mut learning_stream: std::pin::Pin<
        Box<dyn futures::Stream<Item = agent007_learning::LearningEvent> + Send>,
    > = state.learning_dispatcher.subscribe();

    let metrics = state.metrics.clone();
    let mut stats_interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

    let send_task = tokio::spawn(async move {
        loop {
            let msg: Option<WsMessage> = tokio::select! {
                maybe_event = agent_stream.next() => {
                    match maybe_event {
                        Some(event) => {
                            let payload = serde_json::to_value(&event).unwrap_or(Value::Null);
                            Some(WsMessage::AgentEvent {
                                source: "agent".to_string(),
                                payload,
                            })
                        }
                        None => break,
                    }
                }
                maybe_event = learning_stream.next() => {
                    match maybe_event {
                        Some(event) => {
                            let avg_reward: f64 = event.avg_reward();
                            let payload = serde_json::to_value(&event).unwrap_or(Value::Null);
                            Some(WsMessage::LearningEvent { avg_reward, payload })
                        }
                        None => break,
                    }
                }
                _ = stats_interval.tick() => {
                    let snapshot = crate::metrics::snapshot_with_shared_state(
                        metrics.lock().await.clone(),
                        agent007_home(),
                    );
                    Some(WsMessage::StatusUpdate { metrics: snapshot })
                }
            };

            if let Some(m) = msg {
                let text = match serde_json::to_string(&m) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if sender.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        if matches!(msg, Message::Close(_)) {
            break;
        }
    }

    send_task.abort();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::DashboardMetrics;

    #[test]
    fn ws_message_agent_event_serializes() {
        let msg = WsMessage::AgentEvent {
            source: "agent".to_string(),
            payload: serde_json::json!({ "kind": "TaskStarted" }),
        };
        let s = serde_json::to_string(&msg).unwrap();
        assert!(s.contains("agent") || s.contains("AgentEvent"));
    }

    #[test]
    fn ws_message_learning_event_serializes() {
        let msg = WsMessage::LearningEvent {
            avg_reward: 0.75,
            payload: serde_json::json!({ "task_id": "abc" }),
        };
        let s = serde_json::to_string(&msg).unwrap();
        assert!(s.contains("0.75") || s.contains("LearningEvent"));
    }

    #[test]
    fn ws_message_status_update_serializes() {
        let msg = WsMessage::StatusUpdate {
            metrics: DashboardMetrics::new(),
        };
        let s = serde_json::to_string(&msg).unwrap();
        assert!(s.contains("StatusUpdate"));
    }
}
