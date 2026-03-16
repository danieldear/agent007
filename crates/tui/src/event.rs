pub struct EventLoop;

#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    Quit,
    TogglePause,
    Help,
    ScrollUp,
    ScrollDown,
    None,
}
