use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, Tabs, Wrap,
};
use ratatui::{Frame, Terminal};

use crate::config::Config;
use agent007_core::paths::{agent007_global_home, agent007_project_home};
use agent007_core::{AgentMessage, AgentMessageKind, RunMetadata, RunStatus, RunStore};
use agent007_workflows::approval::{ApprovalDecision, ApprovalDecisionKind};
use agent007_workflows::state::PendingApproval;
use agent007_workflows::{WorkflowRunState, WorkflowStepStatus};

#[derive(Args, Debug)]
pub struct TuiArgs {
    /// Number of recent sessions to load into the TUI.
    #[arg(long, short = 'n', default_value_t = 50)]
    pub limit: usize,
    /// Refresh interval in seconds.
    #[arg(long, default_value_t = 5)]
    pub refresh: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiView {
    Sessions,
    Detail,
    Approvals,
    Errors,
}

impl TuiView {
    fn index(self) -> usize {
        match self {
            Self::Sessions => 0,
            Self::Detail => 1,
            Self::Approvals => 2,
            Self::Errors => 3,
        }
    }

    fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Detail,
            2 => Self::Approvals,
            3 => Self::Errors,
            _ => Self::Sessions,
        }
    }
}

#[derive(Debug, Clone)]
struct TuiSession {
    run: RunMetadata,
    workflow: Option<WorkflowRunState>,
    messages: Vec<AgentMessage>,
}

impl TuiSession {
    fn lifecycle(&self) -> &'static str {
        match self.run.status {
            RunStatus::Running => {
                if self
                    .workflow
                    .as_ref()
                    .and_then(|workflow| workflow.last_error.as_ref())
                    .is_some()
                {
                    "attention"
                } else if self.pending_approval().is_some() {
                    "blocked"
                } else {
                    "running"
                }
            }
            RunStatus::AwaitingApproval => "blocked",
            RunStatus::Succeeded => "complete",
            RunStatus::Failed => "failed",
        }
    }

    fn pending_approval(&self) -> Option<&PendingApproval> {
        self.workflow.as_ref()?.pending_approval.as_ref()
    }

    fn workflow_label(&self) -> String {
        if let Some(workflow) = &self.workflow {
            format!(
                "{} {}/{}",
                workflow.workflow, workflow.steps_completed, workflow.steps_total
            )
        } else {
            "—".to_string()
        }
    }

    fn error_text(&self) -> Option<String> {
        self.workflow
            .as_ref()
            .and_then(|workflow| workflow.last_error.clone())
            .or_else(|| {
                if self.run.status == RunStatus::Failed {
                    self.run.output_preview.clone()
                } else {
                    None
                }
            })
    }
}

struct TuiApp {
    store: RunStore,
    sessions: Vec<TuiSession>,
    view: TuiView,
    selected: usize,
    scroll: u16,
    limit: usize,
    refresh: Duration,
    last_refresh: Instant,
    status: String,
    should_quit: bool,
    show_help: bool,
}

impl TuiApp {
    fn new(store: RunStore, limit: usize, refresh_secs: u64) -> Result<Self> {
        let mut app = Self {
            store,
            sessions: Vec::new(),
            view: TuiView::Sessions,
            selected: 0,
            scroll: 0,
            limit: limit.clamp(1, 250),
            refresh: Duration::from_secs(refresh_secs.clamp(2, 300)),
            last_refresh: Instant::now(),
            status: String::new(),
            should_quit: false,
            show_help: false,
        };
        app.refresh_sessions()?;
        Ok(app)
    }

    fn refresh_if_due(&mut self) -> Result<()> {
        if self.last_refresh.elapsed() >= self.refresh {
            self.refresh_sessions()?;
        }
        Ok(())
    }

    fn refresh_sessions(&mut self) -> Result<()> {
        let runs = self.store.list_runs(self.limit)?;
        let selected_id = self.selected_session().map(|s| s.run.id.clone());
        self.sessions = runs
            .into_iter()
            .map(|run| {
                let workflow = self
                    .store
                    .read_json_artifact_optional::<WorkflowRunState>(&run.id, "workflow-state.json")
                    .ok()
                    .flatten();
                let messages = self.store.read_messages(&run.id).unwrap_or_default();
                TuiSession {
                    run,
                    workflow,
                    messages,
                }
            })
            .collect();
        if let Some(id) = selected_id {
            if let Some(index) = self
                .sessions
                .iter()
                .position(|session| session.run.id == id)
            {
                self.selected = index;
            }
        }
        if self.selected >= self.sessions.len() {
            self.selected = self.sessions.len().saturating_sub(1);
        }
        self.last_refresh = Instant::now();
        Ok(())
    }

    fn selected_session(&self) -> Option<&TuiSession> {
        self.sessions.get(self.selected)
    }

    fn approval_sessions(&self) -> Vec<usize> {
        self.sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| session.pending_approval().map(|_| index))
            .collect()
    }

    fn error_sessions(&self) -> Vec<usize> {
        self.sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| session.error_text().map(|_| index))
            .collect()
    }

    fn move_selection(&mut self, delta: isize) {
        let len = match self.view {
            TuiView::Approvals => self.approval_sessions().len(),
            TuiView::Errors => self.error_sessions().len(),
            _ => self.sessions.len(),
        };
        if len == 0 {
            self.selected = 0;
            return;
        }
        let current = self.selected.min(len.saturating_sub(1)) as isize;
        self.selected = (current + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
        self.scroll = 0;
    }

    fn set_view(&mut self, view: TuiView) {
        self.view = view;
        self.selected = 0;
        self.scroll = 0;
    }

    fn selected_index_for_view(&self) -> Option<usize> {
        match self.view {
            TuiView::Approvals => self.approval_sessions().get(self.selected).copied(),
            TuiView::Errors => self.error_sessions().get(self.selected).copied(),
            _ => Some(self.selected).filter(|index| *index < self.sessions.len()),
        }
    }

    fn selected_for_view(&self) -> Option<&TuiSession> {
        self.selected_index_for_view()
            .and_then(|index| self.sessions.get(index))
    }

    fn approve_selected(&mut self, decision: ApprovalDecisionKind) -> Result<()> {
        let Some(index) = self.selected_index_for_view() else {
            self.status = "No selected session.".to_string();
            return Ok(());
        };
        let Some(session) = self.sessions.get(index).cloned() else {
            return Ok(());
        };
        let Some(pending) = session.pending_approval().cloned() else {
            self.status = "Selected session is not awaiting approval.".to_string();
            return Ok(());
        };
        let mut state: WorkflowRunState = self
            .store
            .read_json_artifact(&session.run.id, "workflow-state.json")
            .context("read workflow-state.json")?;
        let content = if decision == ApprovalDecisionKind::Approve {
            Some(pending.content.clone())
        } else {
            None
        };
        state.record_approval_decision(
            &pending.step_id,
            ApprovalDecision {
                decision: decision.clone(),
                content,
            },
        );
        self.store
            .write_json_artifact(&session.run.id, "workflow-state.json", &state)?;
        let _ = self.store.update_run_status(
            &session.run.id,
            RunStatus::Running,
            Some(format!(
                "{} recorded for step {}",
                decision_label(&decision),
                pending.step_id
            )),
        );
        self.store.append_message(
            &session.run.id,
            AgentMessage::new(
                &session.run.id,
                "operator-tui",
                Some(pending.agent.clone()),
                AgentMessageKind::Result,
                format!(
                    "{} recorded for approval step {}",
                    decision_label(&decision),
                    pending.step_id
                ),
                serde_json::json!({ "step_id": pending.step_id, "decision": decision }),
            ),
        )?;
        self.status = format!(
            "{} recorded. Resume session {} to continue.",
            decision_label(&decision),
            short_id(&session.run.id)
        );
        self.refresh_sessions()?;
        Ok(())
    }

    fn request_retry(&mut self) -> Result<()> {
        let Some(session) = self.selected_for_view() else {
            self.status = "No selected session.".to_string();
            return Ok(());
        };
        self.store.append_message(
            &session.run.id,
            AgentMessage::new(
                &session.run.id,
                "operator-tui",
                None,
                AgentMessageKind::Request,
                "Retry requested from operator TUI. Inspect failure and resume/re-run with the appropriate workflow command.",
                serde_json::json!({ "action": "retry-requested" }),
            ),
        )?;
        self.status = format!("Retry request recorded for {}.", short_id(&session.run.id));
        self.refresh_sessions()?;
        Ok(())
    }

    fn write_summary(&mut self) -> Result<()> {
        let summary = self.summary_text();
        let path = agent007_global_home().join("tui-summary.txt");
        std::fs::write(&path, summary)?;
        self.status = format!("Summary written to {}", path.display());
        Ok(())
    }

    fn summary_text(&self) -> String {
        let mut out = String::new();
        out.push_str("agent007 TUI summary\n");
        out.push_str(&format!("generated_at: {}\n", Utc::now().to_rfc3339()));
        out.push_str(&format!("sessions: {}\n", self.sessions.len()));
        let active = self
            .sessions
            .iter()
            .filter(|s| matches!(s.lifecycle(), "running" | "blocked" | "attention"))
            .count();
        let failed = self
            .sessions
            .iter()
            .filter(|s| s.lifecycle() == "failed")
            .count();
        out.push_str(&format!("active: {active}\nfailed: {failed}\n\n"));
        if let Some(session) = self.selected_for_view() {
            out.push_str(&format!("selected: {}\n", session.run.id));
            out.push_str(&format!("task: {}\n", compact_line(&session.run.task)));
            out.push_str(&format!("status: {}\n", session.lifecycle()));
            if let Some(workflow) = &session.workflow {
                out.push_str(&format!(
                    "workflow: {} {}/{}\n",
                    workflow.workflow, workflow.steps_completed, workflow.steps_total
                ));
            }
            if let Some(error) = session.error_text() {
                out.push_str(&format!("error: {}\n", compact_line(&error)));
            }
        }
        out
    }
}

pub async fn execute(_config: Arc<Config>, args: TuiArgs) -> Result<()> {
    let sessions_dir = tui_sessions_dir();
    let store = RunStore::new(sessions_dir);
    let app = TuiApp::new(store, args.limit, args.refresh)?;
    run_terminal(app)
}

fn tui_sessions_dir() -> PathBuf {
    if let Ok(home) = std::env::var("AGENT007_HOME") {
        return PathBuf::from(home).join("sessions");
    }
    if let Some(project_home) = agent007_project_home() {
        return project_home.join("sessions");
    }
    agent007_global_home().join("sessions")
}

fn run_terminal(mut app: TuiApp) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = loop {
        if let Err(error) = app.refresh_if_due() {
            app.status = format!("refresh failed: {error}");
        }
        terminal.draw(|frame| render(frame, &app))?;
        if app.should_quit {
            break Ok(());
        }
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Err(error) = handle_key(&mut app, key.code) {
                        app.status = format!("error: {error}");
                    }
                }
            }
        }
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn handle_key(app: &mut TuiApp, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('?') => app.show_help = !app.show_help,
        KeyCode::Char('1') => app.set_view(TuiView::Sessions),
        KeyCode::Char('2') | KeyCode::Enter => app.set_view(TuiView::Detail),
        KeyCode::Char('3') => app.set_view(TuiView::Approvals),
        KeyCode::Char('4') => app.set_view(TuiView::Errors),
        KeyCode::Tab => app.set_view(TuiView::from_index((app.view.index() + 1) % 4)),
        KeyCode::BackTab => app.set_view(TuiView::from_index((app.view.index() + 3) % 4)),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(8),
        KeyCode::PageDown => app.scroll = app.scroll.saturating_add(8),
        KeyCode::Char('r') => app.refresh_sessions()?,
        KeyCode::Char('a') => app.approve_selected(ApprovalDecisionKind::Approve)?,
        KeyCode::Char('d') => app.approve_selected(ApprovalDecisionKind::Deny)?,
        KeyCode::Char('R') => app.request_retry()?,
        KeyCode::Char('c') => app.write_summary()?,
        _ => {}
    }
    Ok(())
}

fn render(frame: &mut Frame, app: &TuiApp) {
    let area = frame.area();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);
    render_header(frame, vertical[0], app);
    render_tabs(frame, vertical[1], app);
    match app.view {
        TuiView::Sessions => render_sessions(frame, vertical[2], app, None),
        TuiView::Detail => render_detail(frame, vertical[2], app),
        TuiView::Approvals => {
            render_sessions(frame, vertical[2], app, Some(app.approval_sessions()))
        }
        TuiView::Errors => render_sessions(frame, vertical[2], app, Some(app.error_sessions())),
    }
    render_footer(frame, vertical[3], app);
    if app.show_help {
        render_help(frame, area);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let active = app
        .sessions
        .iter()
        .filter(|s| matches!(s.lifecycle(), "running" | "blocked" | "attention"))
        .count();
    let blocked = app
        .sessions
        .iter()
        .filter(|s| s.lifecycle() == "blocked")
        .count();
    let failed = app
        .sessions
        .iter()
        .filter(|s| s.lifecycle() == "failed")
        .count();
    let text = Line::from(vec![
        Span::styled(" AGENT007 ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(format!("operator TUI  active={active} blocked={blocked} failed={failed} sessions={} refresh={}s", app.sessions.len(), app.refresh.as_secs())),
    ]);
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let titles = ["1 Sessions", "2 Detail", "3 Approvals", "4 Errors"]
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    let tabs = Tabs::new(titles)
        .select(app.view.index())
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(tabs, area);
}

fn render_sessions(frame: &mut Frame, area: Rect, app: &TuiApp, indices: Option<Vec<usize>>) {
    let rows = indices.unwrap_or_else(|| (0..app.sessions.len()).collect::<Vec<_>>());
    let table_rows = rows
        .iter()
        .enumerate()
        .filter_map(|(visible_index, session_index)| {
            let session = app.sessions.get(*session_index)?;
            let selected = visible_index == app.selected;
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else {
                lifecycle_style(session.lifecycle())
            };
            Some(
                Row::new(vec![
                    Cell::from(session.lifecycle()),
                    Cell::from(short_id(&session.run.id)),
                    Cell::from(truncate(&session.run.kind, 16)),
                    Cell::from(truncate(&session.workflow_label(), 24)),
                    Cell::from(format_age(
                        (Utc::now() - session.run.started_at).num_seconds().max(0),
                    )),
                    Cell::from(truncate(&compact_line(&session.run.task), 70)),
                ])
                .style(style),
            )
        });
    let title = match app.view {
        TuiView::Approvals => "Approvals Queue",
        TuiView::Errors => "Recent Errors",
        _ => "Sessions",
    };
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(18),
            Constraint::Length(26),
            Constraint::Length(8),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(["state", "session", "kind", "workflow", "age", "task"])
            .style(Style::default().fg(Color::DarkGray)),
    )
    .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(table, area);
}

fn render_detail(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);
    let Some(session) = app.selected_for_view() else {
        frame.render_widget(
            Paragraph::new("No session selected.")
                .block(Block::default().borders(Borders::ALL).title("Detail")),
            area,
        );
        return;
    };
    let mut left = Vec::new();
    left.push(Line::from(format!("id: {}", session.run.id)));
    left.push(Line::from(format!("state: {}", session.lifecycle())));
    left.push(Line::from(format!("kind: {}", session.run.kind)));
    left.push(Line::from(format!(
        "runtime: {}{}",
        session.run.mode,
        session
            .run
            .provider
            .as_ref()
            .map(|p| format!("/{p}"))
            .unwrap_or_default()
    )));
    left.push(Line::from(format!(
        "started: {}",
        session.run.started_at.format("%Y-%m-%d %H:%M:%SZ")
    )));
    left.push(Line::from(""));
    left.push(Line::from("task:"));
    left.push(Line::from(compact_line(&session.run.task)));
    if let Some(workflow) = &session.workflow {
        left.push(Line::from(""));
        left.push(Line::from(format!(
            "workflow: {} {}/{}",
            workflow.workflow, workflow.steps_completed, workflow.steps_total
        )));
        if let Some(pending) = &workflow.pending_approval {
            left.push(Line::from(format!(
                "approval: {} -> {}",
                pending.step_id, pending.agent
            )));
        }
        if let Some(error) = &workflow.last_error {
            left.push(Line::from(format!("error: {}", compact_line(error))));
        }
    }
    frame.render_widget(
        Paragraph::new(left)
            .block(Block::default().borders(Borders::ALL).title("Run Detail"))
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0)),
        chunks[0],
    );

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(chunks[1]);
    render_steps(frame, right_chunks[0], session);
    render_messages(frame, right_chunks[1], session);
}

fn render_steps(frame: &mut Frame, area: Rect, session: &TuiSession) {
    let items = session
        .workflow
        .as_ref()
        .map(|workflow| {
            workflow
                .steps
                .iter()
                .map(|step| {
                    let marker = match step.status {
                        WorkflowStepStatus::Completed => "✓",
                        WorkflowStepStatus::Running => "▶",
                        WorkflowStepStatus::AwaitingApproval => "!",
                        WorkflowStepStatus::Failed => "×",
                        WorkflowStepStatus::Skipped => "-",
                        WorkflowStepStatus::Pending => "·",
                    };
                    ListItem::new(format!(
                        "{marker} {:<18} {:<16} attempts={} {}",
                        step.id,
                        step.agent,
                        step.attempts,
                        step.error.clone().unwrap_or_default()
                    ))
                    .style(step_style(&step.status))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![ListItem::new("No workflow state for this run.")]);
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Workflow Steps"),
        ),
        area,
    );
}

fn render_messages(frame: &mut Frame, area: Rect, session: &TuiSession) {
    let items = if session.messages.is_empty() {
        vec![ListItem::new("No agent messages yet.")]
    } else {
        session
            .messages
            .iter()
            .rev()
            .take(8)
            .map(|message| {
                let to = message
                    .to
                    .as_ref()
                    .map(|value| format!(" -> {value}"))
                    .unwrap_or_default();
                ListItem::new(format!(
                    "[{}] {}{}: {}",
                    message.kind,
                    message.from,
                    to,
                    compact_line(&message.body)
                ))
            })
            .collect()
    };
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Agent Messages"),
        ),
        area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let hint = " q quit · tab switch · ↑↓ select · enter detail · a approve · d deny · R retry note · c copy summary · r refresh · ? help ";
    let status = if app.status.is_empty() {
        hint.to_string()
    } else {
        format!("{} | {}", app.status, hint)
    };
    frame.render_widget(
        Paragraph::new(status).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

fn render_help(frame: &mut Frame, area: Rect) {
    let width = 76u16.min(area.width);
    let height = 18u16.min(area.height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let text = "agent007 operator TUI\n\nViews\n  1 Sessions   recent runtime sessions\n  2 Detail     selected run, workflow steps, messages\n  3 Approvals  sessions waiting for a human decision\n  4 Errors     failed or attention-needed sessions\n\nActions\n  a approve selected approval gate\n  d deny selected approval gate\n  R record retry request message\n  c write compact summary to ~/.agent007/tui-summary.txt\n  r refresh now\n\nApprovals update workflow-state.json; resume the workflow from CLI/MCP to continue.";
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title("Help"),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn lifecycle_style(lifecycle: &str) -> Style {
    match lifecycle {
        "blocked" | "attention" => Style::default().fg(Color::Yellow),
        "failed" => Style::default().fg(Color::Red),
        "complete" => Style::default().fg(Color::DarkGray),
        _ => Style::default().fg(Color::Green),
    }
}

fn step_style(status: &WorkflowStepStatus) -> Style {
    match status {
        WorkflowStepStatus::Completed => Style::default().fg(Color::Green),
        WorkflowStepStatus::Running => Style::default().fg(Color::Cyan),
        WorkflowStepStatus::AwaitingApproval => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        WorkflowStepStatus::Failed => Style::default().fg(Color::Red),
        WorkflowStepStatus::Skipped => Style::default().fg(Color::DarkGray),
        WorkflowStepStatus::Pending => Style::default(),
    }
}

fn decision_label(decision: &ApprovalDecisionKind) -> &'static str {
    match decision {
        ApprovalDecisionKind::Approve => "approval",
        ApprovalDecisionKind::Deny => "denial",
        ApprovalDecisionKind::Edit => "edit",
    }
}

fn short_id(id: &str) -> String {
    truncate(id, 8)
}

fn compact_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

fn format_age(seconds: i64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_filters_approval_and_error_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let store = RunStore::new(tmp.path().join("sessions"));
        let failed = store.create_run("task", "bad", "hosted-mcp", None).unwrap();
        store.finish_run(&failed.id, false, "boom").unwrap();
        let app = TuiApp::new(store, 10, 5).unwrap();
        assert_eq!(app.error_sessions().len(), 1);
    }

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("abcdefgh", 5), "abcd…");
    }

    #[test]
    fn compact_line_normalizes_whitespace() {
        assert_eq!(compact_line("a\n b\tc"), "a b c");
    }
}
