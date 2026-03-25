use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
    response::{Html, IntoResponse},
};
use tokio_util::sync::CancellationToken;

use agent007_core::dispatcher::LocalDispatcher;
use agent007_learning::LearningDispatcher;
use agent007_models::{MockProvider, ModelProvider, ModelRouter};

use crate::api;
use crate::dashboard::DASHBOARD_HTML;
use crate::error::WebError;
use crate::ws;

/// Shared application state passed to every axum handler via `axum::extract::State`.
#[derive(Clone)]
pub struct AppState {
    pub dispatcher: Arc<LocalDispatcher>,
    pub learning_dispatcher: Arc<LearningDispatcher>,
    pub model_router: Arc<ModelRouter>,
    pub cancel: CancellationToken,
}

pub struct WebServer {
    state: AppState,
}

impl WebServer {
    /// Construct a `WebServer` from fully-built components (used by the CLI).
    pub fn new(
        dispatcher: Arc<LocalDispatcher>,
        learning_dispatcher: Arc<LearningDispatcher>,
        model_router: Arc<ModelRouter>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            state: AppState {
                dispatcher,
                learning_dispatcher,
                model_router,
                cancel,
            },
        }
    }

    /// Minimal test constructor — uses MockProvider so no real I/O is required.
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn new_test() -> Self {
        let dispatcher = agent007_core::dispatcher::LocalDispatcher::new(16);
        let learning_dispatcher = Arc::new(LearningDispatcher::new(16));
        let mock = Arc::new(MockProvider::new("test", "mock")) as Arc<dyn ModelProvider>;
        let mut router = ModelRouter::new("mock");
        router.register("mock", mock);
        let model_router = Arc::new(router);
        let cancel = CancellationToken::new();
        Self::new(dispatcher, learning_dispatcher, model_router, cancel)
    }

    /// Build the axum `Router`.
    pub fn into_router(self) -> Router {
        Router::new()
            .route("/", get(dashboard_handler))
            .route("/health", get(health_handler))
            .route("/api/health", get(health_handler))
            .route("/ws", get(ws::ws_handler))
            .route("/api/run", post(api::run_handler))
            .route("/api/skills", get(api::skills_handler))
            .route("/api/skills/run", post(api::skills_run_handler))
            .route("/api/status", get(api::status_handler))
            .with_state(self.state)
    }

    /// Bind to `0.0.0.0:<port>` and serve forever (or until `cancel` fires).
    pub async fn run(self, port: u16) -> Result<(), WebError> {
        let addr = format!("0.0.0.0:{port}");
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| WebError::Bind { addr: addr.clone(), source: e })?;

        tracing::info!("agent007 web server listening on http://{addr}");

        let cancel = self.state.cancel.clone();
        let router = self.into_router();

        axum::serve(listener, router)
            .with_graceful_shutdown(async move { cancel.cancelled().await })
            .await
            .map_err(WebError::Io)?;

        Ok(())
    }
}

async fn dashboard_handler() -> impl IntoResponse {
    Html(DASHBOARD_HTML)
}

async fn health_handler() -> impl IntoResponse {
    axum::Json(serde_json::json!({ "status": "ok", "version": "0.1.0" }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum_test::TestServer;

    #[tokio::test]
    async fn health_check_returns_200() {
        let server = WebServer::new_test();
        let app = server.into_router();
        let test_server = TestServer::new(app).unwrap();
        let response = test_server.get("/health").await;
        response.assert_status_ok();
    }

    #[tokio::test]
    async fn dashboard_returns_html() {
        let server = WebServer::new_test();
        let app = server.into_router();
        let test_server = TestServer::new(app).unwrap();
        let response = test_server.get("/").await;
        response.assert_status_ok();
        let body = response.text();
        assert!(body.contains("agent007"), "dashboard must mention agent007");
        assert!(body.contains("<html"), "response must be HTML");
    }
}
