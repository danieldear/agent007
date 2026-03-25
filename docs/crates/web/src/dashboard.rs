/// Inline single-page dashboard HTML served at `GET /`.
/// Source lives at `static/index.html`; embedded at compile time via `include_str!`.
pub const DASHBOARD_HTML: &str = include_str!("../static/index.html");
