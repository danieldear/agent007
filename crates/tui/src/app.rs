use std::collections::VecDeque;
use agent007_core::types::AgentId;
use agent007_core::events::AgentEvent;
use agent007_learning::types::LearningEvent;

pub struct AgentStatus {
    pub agent_id: AgentId,
    pub name: String,
    pub state: AgentState,
}

pub enum AgentState {
    Active,
    Idle,
    Complete,
}

pub struct TaskStatus {
    pub task_id: uuid::Uuid,
    pub description: String,
    pub assigned_to: Option<AgentId>,
    pub done: bool,
    pub success: bool,
}

pub struct OptimizationSummary {
    pub skill_name: String,
    pub old_reward: f32,
    pub new_reward: f32,
}

pub struct ModelUsage {
    pub provider: String,
    pub token_count: usize,
}

pub struct App {
    pub agents: Vec<AgentStatus>,
    pub tasks: VecDeque<TaskStatus>,
    pub logs: VecDeque<String>,
    pub model_usage: Vec<ModelUsage>,
    pub learning_entries: u32,
    pub avg_reward: f32,
    pub recent_optimizations: Vec<OptimizationSummary>,
    pub paused: bool,
    pub should_quit: bool,
    pub show_help: bool,
    pub log_scroll: usize,
    pub task_scroll: usize,
    log_capacity: usize,
}

impl App {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            tasks: VecDeque::new(),
            logs: VecDeque::new(),
            model_usage: Vec::new(),
            learning_entries: 0,
            avg_reward: 0.0,
            recent_optimizations: Vec::new(),
            paused: false,
            should_quit: false,
            show_help: false,
            log_scroll: 0,
            task_scroll: 0,
            log_capacity: 200,
        }
    }

    pub fn handle_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TaskAssigned { agent_id, task } => {
                // Upsert agent status
                if let Some(existing) = self.agents.iter_mut().find(|a| a.agent_id == agent_id) {
                    existing.state = AgentState::Active;
                } else {
                    self.agents.push(AgentStatus {
                        name: format!("agent-{}", &agent_id.to_string()[..8]),
                        agent_id,
                        state: AgentState::Active,
                    });
                }
                // Push task status
                self.tasks.push_back(TaskStatus {
                    task_id: task.id,
                    description: task.description.clone(),
                    assigned_to: task.assigned_to,
                    done: false,
                    success: false,
                });
            }
            AgentEvent::TaskCompleted { agent_id, result, .. } => {
                // Mark matching task done
                for task in self.tasks.iter_mut() {
                    if task.task_id == result.task_id {
                        task.done = true;
                        task.success = result.success;
                        break;
                    }
                }
                // Set agent to Idle
                if let Some(agent) = self.agents.iter_mut().find(|a| a.agent_id == agent_id) {
                    agent.state = AgentState::Idle;
                }
            }
            AgentEvent::ModelRequest { provider, token_estimate, .. } => {
                if let Some(usage) = self.model_usage.iter_mut().find(|u| u.provider == provider) {
                    usage.token_count += token_estimate;
                } else {
                    self.model_usage.push(ModelUsage {
                        provider,
                        token_count: token_estimate,
                    });
                }
            }
            _ => {}
        }
    }

    pub fn handle_learning_event(&mut self, event: LearningEvent) {
        match event {
            LearningEvent::FeedbackRecorded { reward, .. } => {
                let n = self.learning_entries as f32;
                self.avg_reward = (self.avg_reward * n + reward) / (n + 1.0);
                self.learning_entries += 1;
            }
            LearningEvent::PromptImproved { skill_name, old_reward, new_reward } => {
                self.recent_optimizations.insert(0, OptimizationSummary {
                    skill_name,
                    old_reward,
                    new_reward,
                });
                self.recent_optimizations.truncate(10);
            }
            _ => {}
        }
    }

    pub fn handle_action(&mut self, action: crate::event::AppAction) {
        match action {
            crate::event::AppAction::Quit => self.quit(),
            crate::event::AppAction::TogglePause => self.toggle_pause(),
            crate::event::AppAction::Help => self.show_help = !self.show_help,
            crate::event::AppAction::ScrollUp => {
                self.log_scroll = self.log_scroll.saturating_sub(1);
            }
            crate::event::AppAction::ScrollDown => {
                let max = self.logs.len().saturating_sub(1);
                if self.log_scroll < max { self.log_scroll += 1; }
            }
        }
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn push_log(&mut self, msg: String) {
        self.logs.push_back(msg);
        while self.logs.len() > self.log_capacity {
            self.logs.pop_front();
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_core::task::{Task, TaskResult};

    #[test]
    fn handle_task_assigned_adds_agent_to_list() {
        let mut app = App::default();
        let agent_id = AgentId::new();
        let task = Task::new("test task");
        let event = AgentEvent::TaskAssigned { agent_id: agent_id.clone(), task };
        app.handle_event(event);
        assert_eq!(app.agents.len(), 1);
        assert_eq!(app.agents[0].agent_id, agent_id);
        assert!(matches!(app.agents[0].state, AgentState::Active));
    }

    #[test]
    fn handle_task_completed_marks_task_done() {
        let mut app = App::default();
        let agent_id = AgentId::new();
        let task = Task::new("complete me");
        let task_id = task.id;
        app.handle_event(AgentEvent::TaskAssigned { agent_id: agent_id.clone(), task });
        let result = TaskResult::success(task_id, "done".to_string());
        app.handle_event(AgentEvent::TaskCompleted { agent_id, result, skill_name: None, model: None });
        let task_status = app.tasks.iter().find(|t| t.task_id == task_id).unwrap();
        assert!(task_status.done);
        assert!(task_status.success);
    }

    #[test]
    fn handle_learning_event_updates_learning_panel() {
        let mut app = App::default();
        let agent_id = AgentId::new();
        let event = LearningEvent::FeedbackRecorded { agent_id, reward: 0.8 };
        app.handle_learning_event(event);
        assert_eq!(app.learning_entries, 1);
        assert!((app.avg_reward - 0.8).abs() < 1e-5);
    }

    #[test]
    fn handle_learning_event_prompt_improved_appends_optimization() {
        let mut app = App::default();
        let event = LearningEvent::PromptImproved {
            skill_name: "review-pr".to_string(),
            old_reward: 0.2,
            new_reward: 0.7,
        };
        app.handle_learning_event(event);
        assert_eq!(app.recent_optimizations.len(), 1);
        assert_eq!(app.recent_optimizations[0].skill_name, "review-pr");
    }

    #[test]
    fn paused_flag_toggles() {
        let mut app = App::default();
        assert!(!app.paused);
        app.toggle_pause();
        assert!(app.paused);
        app.toggle_pause();
        assert!(!app.paused);
    }
}
