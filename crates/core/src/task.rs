use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::error::CoreError;
use crate::types::AgentId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub description: String,
    pub assigned_to: Option<AgentId>,
    pub task_type: String,
}

impl Task {
    pub fn new(description: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            description: description.to_string(),
            assigned_to: None,
            task_type: "default".to_string(),
        }
    }

    pub fn with_type(mut self, task_type: &str) -> Self {
        self.task_type = task_type.to_string();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: Uuid,
    pub output: String,
    pub success: bool,
}

impl TaskResult {
    pub fn success(task_id: Uuid, output: String) -> Self {
        Self { task_id, output, success: true }
    }

    pub fn failure(task_id: Uuid, reason: String) -> Self {
        Self { task_id, output: reason, success: false }
    }
}

pub struct TaskQueue {
    sender: tokio::sync::mpsc::Sender<Task>,
}

impl TaskQueue {
    pub fn new(capacity: usize) -> (Self, tokio::sync::mpsc::Receiver<Task>) {
        let (tx, rx) = tokio::sync::mpsc::channel(capacity);
        (Self { sender: tx }, rx)
    }

    pub async fn send(&self, task: Task) -> Result<(), CoreError> {
        self.sender.send(task).await.map_err(|_| CoreError::Disconnected)
    }

    pub fn try_send(&self, task: Task) -> Result<(), CoreError> {
        self.sender.try_send(task).map_err(|_| CoreError::TaskQueueFull)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_queue_send_and_receive() {
        let (queue, mut rx) = TaskQueue::new(8);
        let task = Task::new("write a function");
        queue.send(task).await.unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.description, "write a function");
        assert_eq!(received.task_type, "default");
    }

    #[tokio::test]
    async fn task_queue_respects_capacity_limit() {
        let (queue, _rx) = TaskQueue::new(2);
        queue.send(Task::new("t1")).await.unwrap();
        queue.send(Task::new("t2")).await.unwrap();
        // try_send on a full channel with no receiver should fail
        assert!(queue.try_send(Task::new("t3")).is_err());
    }

    #[test]
    fn task_result_success_sets_flag() {
        let id = uuid::Uuid::new_v4();
        let r = TaskResult::success(id, "output".to_string());
        assert!(r.success);
        assert_eq!(r.task_id, id);
    }

    #[test]
    fn task_result_failure_clears_flag() {
        let id = uuid::Uuid::new_v4();
        let r = TaskResult::failure(id, "oops".to_string());
        assert!(!r.success);
        assert_eq!(r.output, "oops");
    }
}
