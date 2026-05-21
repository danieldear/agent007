use std::sync::Arc;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path as AxumPath, State},
    http::{header, HeaderMap, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};
use tokio_util::sync::CancellationToken;

use agent007_core::dispatcher::LocalDispatcher;
use agent007_learning::LearningDispatcher;
use agent007_models::ModelRouter;
use agent007_workflows::WorkflowRunner;

use crate::api;
use crate::dashboard::{load_dist_file, load_dist_index_html, DASHBOARD_HTML};
use crate::error::WebError;
use crate::extensions_api;
use crate::metrics::{self, MetricsState};
use crate::ws;

const DEFAULT_DASHBOARD_BIND_HOST: &str = "127.0.0.1";
const DEFAULT_MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const AUTH_REALM: &str = r#"Basic realm="agent007 dashboard""#;

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
    pub provider_readiness: api::ProviderReadinessResponse,
    /// Directory name of the project this server is serving (e.g. "my-app").
    pub project_name: String,
    /// Full path to the project root (parent of .agent007/).
    pub project_path: String,
    /// Optional dashboard/API auth token. When present, all non-health routes require auth.
    pub dashboard_auth_token: Option<String>,
}

pub struct WebServer {
    state: AppState,
    max_body_bytes: usize,
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
        let model_provider = model_provider.into();
        let provider_readiness = api::ProviderReadinessResponse::from_runtime(
            standalone_mode,
            runtime_mode.clone(),
            model_provider.clone(),
        );
        Self::new_with_provider_readiness(
            dispatcher,
            learning_dispatcher,
            model_router,
            workflow_runner,
            cancel,
            standalone_mode,
            runtime_mode,
            model_provider,
            provider_readiness,
        )
    }

    pub fn new_with_provider_readiness(
        dispatcher: Arc<LocalDispatcher>,
        learning_dispatcher: Arc<LearningDispatcher>,
        model_router: Arc<ModelRouter>,
        workflow_runner: Option<Arc<WorkflowRunner>>,
        cancel: CancellationToken,
        standalone_mode: bool,
        runtime_mode: impl Into<String>,
        model_provider: impl Into<String>,
        provider_readiness: api::ProviderReadinessResponse,
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

        // Derive project name + path from the write home (project-local .agent007/ parent).
        let write_home = agent007_core::paths::agent007_write_home();
        let project_root = write_home
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| write_home.clone());
        let project_name = project_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let project_path = project_root.display().to_string();

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
                provider_readiness,
                project_name,
                project_path,
                dashboard_auth_token: dashboard_auth_token_from_env(),
            },
            max_body_bytes: dashboard_max_body_bytes_from_env(),
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
        // project_name/project_path are derived inside new() from agent007_write_home()
    }

    #[cfg(test)]
    pub fn new_test_with_auth_token(token: impl Into<String>) -> Self {
        let mut server = Self::new_test();
        server.state.dashboard_auth_token = Some(token.into());
        server
    }

    /// Build the axum `Router`.
    pub fn into_router(self) -> Router {
        let state = self.state;
        let max_body_bytes = self.max_body_bytes;
        let auth_state = state.clone();

        Router::new()
            .route("/", get(dashboard_handler))
            .route("/health", get(health_handler))
            .route("/api/health", get(health_handler))
            .route("/ws", get(ws::ws_handler))
            .route("/api/run", post(api::run_handler))
            .route(
                "/api/skills",
                get(api::skills_handler).post(api::skill_save_handler),
            )
            .route(
                "/api/skills/{trigger}",
                axum::routing::delete(api::skill_delete_handler),
            )
            .route("/api/skills/run", post(api::skills_run_handler))
            .route("/api/skills/import", post(api::skill_import_handler))
            .route("/api/skills/preview", post(api::skill_preview_handler))
            .route("/api/skills/discover", post(api::skill_discover_handler))
            .route("/api/skills/generate", post(api::skill_generate_handler))
            .route("/api/skills/detail/{trigger}", get(api::skill_get_handler))
            .route("/api/skill-registry", get(api::skill_registry_handler))
            .route("/api/status", get(api::status_handler))
            .route("/api/stats", get(api::stats_handler))
            .route("/api/scorecards", get(api::scorecards_handler))
            .route(
                "/api/regression/evaluate",
                get(api::regression_evaluate_handler),
            )
            .route("/api/runtime/sessions", get(api::runtime_sessions_handler))
            .route(
                "/api/runtime/sessions/{id}/messages",
                get(api::runtime_messages_list_handler).post(api::runtime_messages_post_handler),
            )
            .route("/api/providers/status", get(api::provider_status_handler))
            .route("/api/runs", get(api::runs_handler))
            .route(
                "/api/runs/cleanup-awaiting",
                post(api::runs_cleanup_awaiting_handler),
            )
            .route("/api/runs/{id}", get(api::run_detail_handler))
            .route(
                "/api/runs/{id}/artifacts/preview",
                get(api::run_artifact_preview_handler),
            )
            .route(
                "/api/runs/{id}/artifacts/raw",
                get(api::run_artifact_raw_handler),
            )
            .route("/api/runs/{id}/approval", post(api::run_approval_handler))
            .route("/api/runs/{id}/resume", post(api::run_resume_handler))
            .route(
                "/api/personas",
                get(api::personas_list_handler).post(api::persona_save_handler),
            )
            .route(
                "/api/personas/{name}",
                axum::routing::delete(api::persona_delete_handler),
            )
            .route(
                "/api/workflows",
                get(api::workflows_list_handler).post(api::workflow_save_handler),
            )
            .route(
                "/api/workflows/validate",
                post(api::workflow_validate_handler),
            )
            .route(
                "/api/workflows/{name}",
                get(api::workflow_get_handler).delete(api::workflow_delete_handler),
            )
            .route(
                "/api/workflow-templates",
                get(api::workflow_templates_list_handler),
            )
            .route(
                "/api/workflow-templates/{name}",
                get(api::workflow_template_get_handler),
            )
            .route("/api/memory/{scope}", get(api::memory_list_handler))
            .route("/api/memory/{scope}/stats", get(api::memory_stats_handler))
            .route(
                "/api/memory/{scope}/_actions/purge-expired",
                post(api::memory_purge_expired_handler),
            )
            .route(
                "/api/memory/{scope}/{key}",
                get(api::memory_get_handler).delete(api::memory_delete_handler),
            )
            .route(
                "/api/skills/{trigger}/promote",
                post(api::skill_promote_handler),
            )
            .route(
                "/api/workflows/{name}/promote",
                post(api::workflow_promote_handler),
            )
            .route(
                "/api/tools",
                get(api::tools_list_handler).post(api::tool_save_handler),
            )
            .route("/api/tools/search", get(api::tools_search_handler))
            .route("/api/tools/discover", get(api::tools_discover_handler))
            .route("/api/tools/import", post(api::tool_import_handler))
            .route(
                "/api/tools/{name}",
                get(api::tool_get_handler).delete(api::tool_delete_handler),
            )
            .route("/api/tools/{name}/test", post(api::tool_test_handler))
            .route("/api/tools/{name}/approve", post(api::tool_approve_handler))
            .route("/api/scripts", get(api::scripts_list_handler))
            .route("/api/bundle/export", get(api::bundle_export_handler))
            .route("/api/bundle/import", post(api::bundle_import_handler))
            // MCP server registry
            .route(
                "/api/mcp/servers",
                get(api::mcp_list_handler).post(api::mcp_add_handler),
            )
            .route("/api/mcp/servers/{name}", delete(api::mcp_delete_handler))
            .route(
                "/api/mcp/servers/{name}/connect",
                post(api::mcp_connect_handler),
            )
            .route(
                "/api/mcp/servers/{name}/approve",
                post(api::mcp_approve_handler),
            )
            .route("/api/mcp/servers/{name}/tools", get(api::mcp_tools_handler))
            .route(
                "/api/lsp/config",
                get(api::lsp_config_get_handler)
                    .post(api::lsp_config_set_handler)
                    .delete(api::lsp_config_delete_handler),
            )
            // RAG sources
            .route(
                "/api/rag/sources",
                get(api::rag_list_handler).post(api::rag_add_handler),
            )
            .route(
                "/api/rag/sources/{id}/reindex",
                post(api::rag_reindex_handler),
            )
            .route("/api/rag/sources/{id}", delete(api::rag_delete_handler))
            .route("/api/rag/query", get(api::rag_query_handler))
            // Extensions
            .route(
                "/api/extensions/preview",
                post(extensions_api::preview_handler),
            )
            .route(
                "/api/extensions/install",
                post(extensions_api::install_handler),
            )
            .route(
                "/api/extensions/uninstall",
                post(extensions_api::uninstall_handler),
            )
            .route("/api/extensions/list", get(extensions_api::list_handler))
            .route("/api/etr/tools", get(api::etr_list_handler))
            .route("/api/etr/call", post(api::etr_call_handler))
            .route("/api/etr/cache/stats", get(api::etr_cache_stats_handler))
            .route("/api/etr/cache/clear", post(api::etr_cache_clear_handler))
            .route("/assets/{*path}", get(asset_handler))
            .layer(DefaultBodyLimit::max(max_body_bytes))
            .layer(middleware::from_fn_with_state(
                auth_state,
                require_dashboard_auth,
            ))
            .with_state(state)
    }

    /// Bind to the configured dashboard host and serve forever (or until `cancel` fires).
    ///
    /// Defaults to `127.0.0.1:<port>` so the dashboard is not exposed on the LAN
    /// unless the operator explicitly sets `AGENT007_DASHBOARD_HOST`.
    pub async fn run(self, port: u16) -> Result<(), WebError> {
        let addr = dashboard_bind_addr(port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| WebError::Bind {
                addr: addr.clone(),
                source: e,
            })?;

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
    pub async fn run_with_listener(
        self,
        listener: tokio::net::TcpListener,
    ) -> Result<(), WebError> {
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
            let addr = dashboard_bind_addr(port);
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

/// Dashboard bind host. Defaults to localhost for safety.
pub fn dashboard_bind_host() -> String {
    std::env::var("AGENT007_DASHBOARD_HOST")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_DASHBOARD_BIND_HOST.to_string())
}

/// Dashboard bind address for a port.
pub fn dashboard_bind_addr(port: u16) -> String {
    format!("{}:{port}", dashboard_bind_host())
}

fn dashboard_auth_token_from_env() -> Option<String> {
    std::env::var("AGENT007_DASHBOARD_AUTH_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn dashboard_max_body_bytes_from_env() -> usize {
    std::env::var("AGENT007_DASHBOARD_MAX_BODY_BYTES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAX_BODY_BYTES)
}

async fn require_dashboard_auth(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(expected) = state.dashboard_auth_token.as_deref() else {
        return next.run(request).await;
    };

    let path = request.uri().path();
    if path == "/health" || path == "/api/health" {
        return next.run(request).await;
    }

    if request_is_authorized(request.headers(), expected) {
        return next.run(request).await;
    }

    let mut response = StatusCode::UNAUTHORIZED.into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static(AUTH_REALM),
    );
    response
}

fn request_is_authorized(headers: &HeaderMap, expected: &str) -> bool {
    if let Some(value) = headers
        .get("x-agent007-token")
        .and_then(|v| v.to_str().ok())
    {
        if constant_time_eq(value.as_bytes(), expected.as_bytes()) {
            return true;
        }
    }

    let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };

    if let Some(token) = auth.strip_prefix("Bearer ") {
        return constant_time_eq(token.trim().as_bytes(), expected.as_bytes());
    }

    if let Some(encoded) = auth.strip_prefix("Basic ") {
        return basic_auth_password_matches(encoded.trim(), expected);
    }

    false
}

fn basic_auth_password_matches(encoded: &str, expected: &str) -> bool {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    let Ok(decoded) = STANDARD.decode(encoded) else {
        return false;
    };
    let Ok(decoded) = String::from_utf8(decoded) else {
        return false;
    };
    let Some((_, password)) = decoded.split_once(':') else {
        return false;
    };
    constant_time_eq(password.as_bytes(), expected.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    let max_len = a.len().max(b.len());
    for i in 0..max_len {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        diff |= (av ^ bv) as usize;
    }
    diff == 0
}

async fn dashboard_handler() -> impl IntoResponse {
    let html = load_dist_index_html().unwrap_or_else(|| DASHBOARD_HTML.to_string());
    (
        [
            ("cache-control", "no-cache, no-store, must-revalidate"),
            ("pragma", "no-cache"),
        ],
        Html(html),
    )
}

async fn asset_handler(AxumPath(path): AxumPath<String>) -> impl IntoResponse {
    let rel_path = format!("assets/{path}");
    let Some(bytes) = load_dist_file(&rel_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mut headers = HeaderMap::new();
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    let content_type = HeaderValue::from_str(mime.as_ref())
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );

    (headers, bytes).into_response()
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

    #[tokio::test]
    async fn health_check_bypasses_dashboard_auth() {
        let server = WebServer::new_test_with_auth_token("secret");
        let app = server.into_router();
        let test_server = TestServer::new(app).unwrap();
        let response = test_server.get("/health").await;
        response.assert_status_ok();
    }

    #[tokio::test]
    async fn dashboard_auth_rejects_missing_token() {
        let server = WebServer::new_test_with_auth_token("secret");
        let app = server.into_router();
        let test_server = TestServer::new(app).unwrap();
        let response = test_server.get("/").await;
        response.assert_status(StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn dashboard_auth_accepts_bearer_token() {
        let server = WebServer::new_test_with_auth_token("secret");
        let app = server.into_router();
        let test_server = TestServer::new(app).unwrap();
        let response = test_server
            .get("/")
            .add_header(header::AUTHORIZATION, "Bearer secret")
            .await;
        response.assert_status_ok();
    }

    #[tokio::test]
    async fn dashboard_auth_accepts_basic_password() {
        let server = WebServer::new_test_with_auth_token("secret");
        let app = server.into_router();
        let test_server = TestServer::new(app).unwrap();
        let response = test_server
            .get("/")
            .add_header(header::AUTHORIZATION, "Basic YWdlbnQwMDc6c2VjcmV0")
            .await;
        response.assert_status_ok();
    }

    #[test]
    fn default_dashboard_bind_addr_is_localhost() {
        std::env::remove_var("AGENT007_DASHBOARD_HOST");
        assert_eq!(dashboard_bind_addr(8007), "127.0.0.1:8007");
    }
}
