use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
    response::{Html, IntoResponse},
};
use tokio_util::sync::CancellationToken;

use agent007_core::dispatcher::LocalDispatcher;
use agent007_learning::LearningDispatcher;
use agent007_models::ModelRouter;
use agent007_workflows::WorkflowRunner;

use crate::api;
use crate::dashboard::DASHBOARD_HTML;
use crate::error::WebError;
use crate::metrics::{self, MetricsState};
use crate::ws;

/// Shared application state passed to every axum handler via `axum::extract::State`.
#[derive(Clone)]
pub struct AppState {
    pub dispatcher: Arc<LocalDispatcher>,
    pub learning_dispatcher: Arc<LearningDispatcher>,
    pub model_router: Arc<ModelRouter>,
    pub workflow_runner: Option<Arc<WorkflowRunner>>,
    pub cancel: CancellationToken,
    pub metrics: MetricsState,
    pub standalone_mode: bool,
    pub runtime_mode: String,
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
        workflow_runner: Option<Arc<WorkflowRunner>>,
        cancel: CancellationToken,
        standalone_mode: bool,
        runtime_mode: impl Into<String>,
        model_provider: impl Into<String>,
    ) -> Self {
        let runtime_mode = runtime_mode.into();
        let metrics_state = metrics::new_metrics_state_with_runtime(
            standalone_mode,
            runtime_mode.clone(),
            model_provider,
        );
        metrics::spawn_metrics_collector(
            metrics_state.clone(),
            dispatcher.clone(),
            learning_dispatcher.clone(),
        );
        Self {
            state: AppState {
                dispatcher,
                learning_dispatcher,
                model_router,
                workflow_runner,
                cancel,
                metrics: metrics_state,
                standalone_mode,
                runtime_mode,
            },
        }
    }

    /// Minimal test constructor — uses MockProvider so no real I/O is required.
    #[cfg(test)]
    pub fn new_test() -> Self {
        use agent007_core::persona::NoOpPersonaProvider;
        use agent007_models::{MockProvider, ModelProvider};

        let dispatcher = agent007_core::dispatcher::LocalDispatcher::new(16);
        let learning_dispatcher = Arc::new(LearningDispatcher::new(16));
        let mock = Arc::new(MockProvider::new("test", "mock")) as Arc<dyn ModelProvider>;
        let mut router_m = ModelRouter::new("mock");
        router_m.register("mock", mock);
        let model_router = Arc::new(router_m);
        let workflow_runner = Some(Arc::new(WorkflowRunner::new(
            Arc::new(NoOpPersonaProvider),
            model_router.clone(),
            dispatcher.clone() as Arc<dyn agent007_core::dispatcher::Dispatcher>,
        )));
        let cancel = CancellationToken::new();
        Self::new(
            dispatcher,
            learning_dispatcher,
            model_router,
            workflow_runner,
            cancel,
            true,
            "dry-run",
            "mock",
        )
    }

    /// Build the axum `Router`.
    pub fn into_router(self) -> Router {
        Router::new()
            .route("/", get(dashboard_handler))
            .route("/health", get(health_handler))
            .route("/api/health", get(health_handler))
            .route("/ws", get(ws::ws_handler))
            .route("/api/run", post(api::run_handler))
            .route("/api/skills", get(api::skills_handler).post(api::skill_save_handler))
            .route("/api/skills/run", post(api::skills_run_handler))
            .route("/api/skills/import", post(api::skill_import_handler))
            .route("/api/skills/generate", post(api::skill_generate_handler))
            .route("/api/skills/detail/{trigger}", get(api::skill_get_handler))
            .route("/api/skill-registry", get(api::skill_registry_handler))
            .route("/api/status", get(api::status_handler))
            .route("/api/stats", get(api::stats_handler))
            .route("/api/runs", get(api::runs_handler))
            .route("/api/runs/{id}", get(api::run_detail_handler))
            .route("/api/runs/{id}/approval", post(api::run_approval_handler))
            .route("/api/runs/{id}/resume", post(api::run_resume_handler))
            .route("/api/personas", get(api::personas_list_handler).post(api::persona_save_handler))
            .route("/api/personas/{name}", axum::routing::delete(api::persona_delete_handler))
            .route("/api/workflows", get(api::workflows_list_handler).post(api::workflow_save_handler))
            .route("/api/workflows/validate", post(api::workflow_validate_handler))
            .route("/api/workflows/{name}", get(api::workflow_get_handler))
            .route("/api/workflow-templates", get(api::workflow_templates_list_handler))
            .route("/api/workflow-templates/{name}", get(api::workflow_template_get_handler))
            .route("/api/memory/{scope}", get(api::memory_list_handler))
            .nest_service("/assets", {
                let dist_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("static/dist/assets");
                tower_http::services::ServeDir::new(dist_dir)
            })
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

    /// Start serving using a pre-bound `TcpListener`.
    /// Use this to avoid the TOCTOU race when the caller already holds the binding.
    pub async fn run_with_listener(self, listener: tokio::net::TcpListener) -> Result<(), WebError> {
        let cancel = self.state.cancel.clone();
        let router = self.into_router();
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { cancel.cancelled().await })
            .await
            .map_err(WebError::Io)?;
        Ok(())
    }

    /// Try `preferred_port`, then increment up to 50 times until a free port is found.
    /// Returns the actual port the server bound to.
    pub async fn run_auto_port(self, preferred_port: u16) -> Result<u16, WebError> {
        let max_attempts = 50;
        let cancel = self.state.cancel.clone();
        let router = self.into_router();

        for offset in 0..max_attempts {
            let port = preferred_port.wrapping_add(offset);
            let addr = format!("0.0.0.0:{port}");
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(listener) => {
                    tracing::info!("agent007 web server listening on http://{addr}");
                    let cancel = cancel.clone();
                    axum::serve(listener, router)
                        .with_graceful_shutdown(async move { cancel.cancelled().await })
                        .await
                        .map_err(WebError::Io)?;
                    return Ok(port);
                }
                Err(_) if offset < max_attempts - 1 => continue,
                Err(e) => {
                    return Err(WebError::Bind { addr, source: e });
                }
            }
        }
        unreachable!()
    }
}

async fn dashboard_handler() -> impl IntoResponse {
    let dist_index = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("static/dist/index.html");
    let html = if dist_index.exists() {
        std::fs::read_to_string(&dist_index).unwrap_or_else(|_| DASHBOARD_HTML.to_string())
    } else {
        DASHBOARD_HTML.to_string()
    };
    (
        [
            ("cache-control", "no-cache, no-store, must-revalidate"),
            ("pragma", "no-cache"),
        ],
        Html(html),
    )
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
