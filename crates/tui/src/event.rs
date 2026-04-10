use crossterm::event::KeyCode;
use agent007_core::events::AgentEvent;
use agent007_learning::types::LearningEvent;
use crate::app::App;
use crate::TuiError;

#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    Quit,
    TogglePause,
    Help,
    ScrollUp,
    ScrollDown,
}

/// Maps a crossterm KeyCode to an AppAction (or None for unbound keys).
/// q → Quit, p → TogglePause, ? / Esc → Help, ↑↓ → Scroll
pub fn map_key_event(key: KeyCode) -> Option<AppAction> {
    match key {
        KeyCode::Char('q') => Some(AppAction::Quit),
        KeyCode::Char('p') => Some(AppAction::TogglePause),
        KeyCode::Char('?') | KeyCode::Esc => Some(AppAction::Help),
        KeyCode::Up => Some(AppAction::ScrollUp),
        KeyCode::Down => Some(AppAction::ScrollDown),
        _ => None,
    }
}

pub struct EventLoop {
    agent_event_rx: tokio::sync::mpsc::Receiver<AgentEvent>,
    learning_event_rx: tokio::sync::mpsc::Receiver<LearningEvent>,
}

impl EventLoop {
    /// Construct by subscribing to both dispatchers.
    /// Spawns two background tasks that forward events into mpsc channels.
    pub async fn new(
        dispatcher: std::sync::Arc<dyn agent007_core::dispatcher::Dispatcher>,
        learning_dispatcher: std::sync::Arc<agent007_learning::LearningDispatcher>,
    ) -> Result<Self, TuiError> {
        use futures::StreamExt as _;
        let (agent_tx, agent_event_rx) = tokio::sync::mpsc::channel::<AgentEvent>(1024);
        let (learning_tx, learning_event_rx) = tokio::sync::mpsc::channel::<LearningEvent>(1024);

        let mut agent_stream = dispatcher
            .subscribe()
            .await
            .map_err(|e| TuiError::Subscribe(e.to_string()))?;

        tokio::spawn(async move {
            while let Some(event) = agent_stream.next().await {
                if agent_tx.send(event).await.is_err() {
                    break;
                }
            }
        });

        let mut learning_stream = learning_dispatcher.subscribe();

        tokio::spawn(async move {
            while let Some(event) = learning_stream.next().await {
                if learning_tx.send(event).await.is_err() {
                    break;
                }
            }
        });

        Ok(EventLoop {
            agent_event_rx,
            learning_event_rx,
        })
    }

    /// Run the terminal event loop until app.should_quit is true.
    pub async fn run(
        mut self,
        app: &mut App,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), TuiError> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::EnterAlternateScreen
        )?;

        let backend = ratatui::backend::CrosstermBackend::new(std::io::stderr());
        let mut terminal = ratatui::Terminal::new(backend)?;

        let result = self.run_inner(app, cancel, &mut terminal).await;

        // Cleanup — always runs even on error
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::LeaveAlternateScreen
        );

        result
    }

    async fn run_inner(
        &mut self,
        app: &mut App,
        cancel: tokio_util::sync::CancellationToken,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stderr>>,
    ) -> Result<(), TuiError> {
        use crossterm::event::{Event, EventStream};
        use futures::StreamExt as FuturesStreamExt;

        let mut crossterm_events = EventStream::new();

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    break;
                }
                maybe_event = FuturesStreamExt::next(&mut crossterm_events) => {
                    if let Some(Ok(Event::Key(key_event))) = maybe_event {
                        if let Some(action) = map_key_event(key_event.code) {
                            app.handle_action(action);
                        }
                    }
                }
                maybe_agent = self.agent_event_rx.recv() => {
                    if let Some(event) = maybe_agent {
                        if !app.paused {
                            app.handle_event(event);
                        }
                    }
                }
                maybe_learning = self.learning_event_rx.recv() => {
                    if let Some(event) = maybe_learning {
                        if !app.paused {
                            app.handle_learning_event(event);
                        }
                    }
                }
            }

            terminal.draw(|f| crate::ui::render(f, app))?;

            if app.should_quit {
                cancel.cancel();
                break;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;

    #[tokio::test]
    async fn quit_action_sets_should_quit() {
        let mut app = App::default();
        app.handle_action(AppAction::Quit);
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn pause_action_toggles_paused() {
        let mut app = App::default();
        app.handle_action(AppAction::TogglePause);
        assert!(app.paused);
        app.handle_action(AppAction::TogglePause);
        assert!(!app.paused);
    }

    #[test]
    fn key_event_q_maps_to_quit_action() {
        use crossterm::event::KeyCode;
        let action = map_key_event(KeyCode::Char('q'));
        assert_eq!(action, Some(AppAction::Quit));
    }

    #[test]
    fn key_event_p_maps_to_pause_action() {
        use crossterm::event::KeyCode;
        let action = map_key_event(KeyCode::Char('p'));
        assert_eq!(action, Some(AppAction::TogglePause));
    }

    #[test]
    fn unknown_key_maps_to_none() {
        use crossterm::event::KeyCode;
        let action = map_key_event(KeyCode::Char('z'));
        assert_eq!(action, None);
    }
}
