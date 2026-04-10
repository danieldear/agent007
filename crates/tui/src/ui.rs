use crate::app::{AgentState, App};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

/// Entry point called from EventLoop's terminal.draw() closure.
pub fn render(frame: &mut Frame, app: &App) {
    let size = frame.area();

    // Vertical split: header (3 lines) + body (remaining)
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(size);

    let header_area = vertical[0];
    let body_area = vertical[1];

    render_header(frame, header_area, app);

    // Body split into 3 equal-height rows
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(body_area);

    // Each row split: left 30% | right 70%
    let row1 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(3, 10), Constraint::Ratio(7, 10)])
        .split(rows[0]);

    let row2 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(3, 10), Constraint::Ratio(7, 10)])
        .split(rows[1]);

    let row3 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(3, 10), Constraint::Ratio(7, 10)])
        .split(rows[2]);

    render_agents(frame, row1[0], app);
    render_task_queue(frame, row1[1], app);
    render_model(frame, row2[0], app);
    render_logs(frame, row2[1], app);
    render_learning(frame, row3[0], app);
    render_optimizations(frame, row3[1], app);

    if app.show_help {
        render_help_overlay(frame, size);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let pause_hint = if app.paused { " [PAUSED]" } else { "" };
    let block = Block::default()
        .borders(Borders::ALL)
        .title("agent007 v0.1.0");
    let paragraph =
        Paragraph::new(format!("[q]uit [p]ause [?]help ↑↓ scroll logs{pause_hint}")).block(block);
    frame.render_widget(paragraph, area);
}

fn render_agents(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("Agents");
    let items: Vec<ListItem> = app
        .agents
        .iter()
        .map(|a| {
            let bullet = match a.state {
                AgentState::Active => "●",
                AgentState::Idle | AgentState::Complete => "○",
            };
            let style = match a.state {
                AgentState::Active => Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
                AgentState::Complete => Style::default().fg(Color::DarkGray),
                AgentState::Idle => Style::default(),
            };
            ListItem::new(format!("{} {}", bullet, a.name)).style(style)
        })
        .collect();
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_task_queue(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("Task Queue");
    let items: Vec<ListItem> = app
        .tasks
        .iter()
        .map(|t| {
            let prefix = if t.done {
                "[✓]"
            } else if t.assigned_to.is_some() {
                "[→]"
            } else {
                "[ ]"
            };
            let style = if t.done {
                Style::default().fg(Color::DarkGray)
            } else if t.assigned_to.is_some() {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            ListItem::new(format!("{} {}", prefix, t.description)).style(style)
        })
        .collect();
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_model(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("Model Usage");
    let text: String = app
        .model_usage
        .iter()
        .map(|u| format!("{}: {} tokens", u.provider, u.token_count))
        .collect::<Vec<_>>()
        .join("\n");
    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
}

fn render_logs(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Logs (↑↓ scroll)");
    let logs: Vec<&str> = app.logs.iter().map(|s| s.as_str()).collect();
    let skip = app.log_scroll.min(logs.len().saturating_sub(1));
    let text = logs[skip..].join("\n");
    let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_learning(frame: &mut Frame, area: Rect, app: &App) {
    // Split area: top for paragraph, bottom for gauge
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    let block = Block::default().borders(Borders::ALL).title("Learning");
    let text = format!(
        "Entries: {}  Avg Reward: {:.2}",
        app.learning_entries, app.avg_reward
    );
    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, chunks[0]);

    let ratio = if app.learning_entries == 0 {
        0.0
    } else {
        (app.avg_reward as f64).clamp(0.0, 1.0)
    };
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Avg Reward"))
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(ratio);
    frame.render_widget(gauge, chunks[1]);
}

fn render_optimizations(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Optimizations");
    let items: Vec<ListItem> = app
        .recent_optimizations
        .iter()
        .map(|o| {
            ListItem::new(format!(
                "{}: {:.2}→{:.2}",
                o.skill_name, o.old_reward, o.new_reward
            ))
        })
        .collect();
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_help_overlay(frame: &mut Frame, area: Rect) {
    // Center a 50×14 popup
    let popup_w = 52u16.min(area.width);
    let popup_h = 14u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect {
        x,
        y,
        width: popup_w,
        height: popup_h,
    };

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title("Help");
    let text = "\
  q          Quit\n\
  p          Pause / resume event processing\n\
  ? / Esc    Toggle this help overlay\n\
  ↑ / ↓      Scroll log panel\n\
\n\
  All agent007 events are streamed live.\n\
  Logs auto-scroll; use ↑↓ to review history.";
    let paragraph = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, popup);
}

mod tests {
    use super::*;
    use crate::app::App;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn render_with_populated_app_does_not_panic() {
        use agent007_core::events::AgentEvent;
        use agent007_core::task::{Task, TaskResult};
        use agent007_core::types::{AgentId, PromptRef};
        use agent007_learning::types::LearningEvent;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::default();

        // Add 2 agents with tasks
        let agent1 = AgentId::new();
        let agent2 = AgentId::new();
        let task1 = Task::new("Implement feature A");
        let task2 = Task::new("Write tests for module B");
        let task1_id = task1.id;

        app.handle_event(AgentEvent::TaskAssigned {
            agent_id: agent1.clone(),
            task: task1,
        });
        app.handle_event(AgentEvent::TaskAssigned {
            agent_id: agent2.clone(),
            task: task2,
        });

        // Complete task1 so we have a mix of done/active
        let result = TaskResult::success(task1_id, "done".to_string());
        app.handle_event(AgentEvent::TaskCompleted {
            agent_id: agent1,
            result,
            skill_name: None,
            model: None,
        });

        // Model usage
        app.handle_event(AgentEvent::ModelRequest {
            provider: "claude".to_string(),
            prompt_ref: PromptRef::new(),
            token_estimate: 512,
        });

        // 3 log lines
        app.push_log("Starting task: Implement feature A".to_string());
        app.push_log("Agent assigned to task".to_string());
        app.push_log("Task completed successfully".to_string());

        // 1 optimization via learning events
        app.handle_learning_event(LearningEvent::FeedbackRecorded {
            agent_id: agent2.clone(),
            reward: 0.75,
        });
        app.handle_learning_event(LearningEvent::PromptImproved {
            skill_name: "code-review".to_string(),
            old_reward: 0.4,
            new_reward: 0.75,
        });

        terminal.draw(|f| render(f, &app)).unwrap();
    }

    #[test]
    fn render_empty_app_does_not_panic() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = App::default();
        terminal.draw(|f| render(f, &app)).unwrap();
    }
}
