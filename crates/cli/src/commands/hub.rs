use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent007_core::{RunStatus, RunStore};
use agent007_packs::{LockedPack, PackInspection, PackManager, RegistryPack, DEFAULT_REGISTRY_URL};
use anyhow::{Context, Result};
use axum::extract::{Path as AxumPath, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use super::credentials;
use super::hub_assets::{
    AssetDocument, AssetError, AssetKind, AssetSummary, GlobalAssetStore, ValidationResult,
    VersionBump,
};
use super::projects::{default_registry_path, load_registry, project_statuses, ProjectStatus};

const THREAD_LIMIT_PER_PROJECT: usize = 40;

#[derive(Clone)]
struct HubState {
    registry_path: PathBuf,
    ports_path: PathBuf,
    global_home: PathBuf,
    pack_registry: String,
    mutation_token: String,
}

#[derive(Debug, Serialize)]
struct HubThread {
    id: String,
    kind: String,
    task: String,
    mode: String,
    provider: String,
    started_at: String,
    finished_at: Option<String>,
    status: String,
    output_preview: Option<String>,
}

#[derive(Debug, Serialize)]
struct HubProject {
    #[serde(flatten)]
    status: ProjectStatus,
    dashboard_port: Option<u16>,
    dashboard_url: Option<String>,
    thread_count: usize,
    active_thread_count: usize,
    threads_error: Option<String>,
    threads: Vec<HubThread>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct HubSummary {
    projects: usize,
    ready_projects: usize,
    running_dashboards: usize,
    threads: usize,
    active_threads: usize,
}

#[derive(Debug, Default, Serialize)]
struct AssetInventory {
    skills: Vec<AssetSummary>,
    workflows: Vec<AssetSummary>,
    personas: Vec<AssetSummary>,
}

#[derive(Debug, Serialize)]
struct MemoryInventory {
    path: String,
    database_present: bool,
    database_size_bytes: u64,
    storage_groups: Vec<String>,
    management_mode: &'static str,
}

#[derive(Debug, Serialize)]
struct ProviderReadiness {
    id: &'static str,
    name: &'static str,
    category: &'static str,
    configured: bool,
    credential_editable: bool,
    credential_managed: bool,
    source: String,
    detail: String,
}

#[derive(Debug, Serialize)]
struct HubResponse {
    generated_at: String,
    registry_path: String,
    global_home: String,
    summary: HubSummary,
    projects: Vec<HubProject>,
    assets: AssetInventory,
    memory: MemoryInventory,
    providers: Vec<ProviderReadiness>,
    capabilities: Value,
}

#[derive(Debug, Default, Deserialize)]
struct PortRegistryFile {
    #[serde(default)]
    projects: HashMap<String, u16>,
}

#[derive(Debug, Deserialize)]
struct ValidateAssetRequest {
    format: Option<String>,
    content: String,
}

#[derive(Debug, Deserialize)]
struct CreateAssetRequest {
    id: String,
    format: Option<String>,
    content: String,
}

#[derive(Debug, Deserialize)]
struct UpdateAssetRequest {
    expected_revision: String,
    bump: VersionBump,
    content: String,
}

#[derive(Debug, Deserialize)]
struct DeleteAssetRequest {
    expected_revision: String,
    confirmation: String,
}

#[derive(Debug, Deserialize)]
struct SetCredentialRequest {
    api_key: String,
}

#[derive(Debug, Default, Deserialize)]
struct PackMutationRequest {
    version: Option<String>,
    #[serde(default)]
    refresh: bool,
}

#[derive(Debug, Serialize)]
struct HubPack {
    id: String,
    name: String,
    description: String,
    categories: Vec<String>,
    versions: Vec<String>,
    latest_version: Option<String>,
    installed_version: Option<String>,
    enabled: bool,
    update_available: bool,
}

#[derive(Debug, Serialize)]
struct HubPackInventory {
    registry: String,
    from_cache: bool,
    packs: Vec<HubPack>,
}

pub async fn execute(port: u16, open_browser: bool) -> Result<()> {
    let global_home = agent007_core::paths::agent007_global_home();
    let state = HubState {
        registry_path: default_registry_path(),
        ports_path: global_home.join("ports.toml"),
        global_home,
        pack_registry: std::env::var("AGENT007_PACK_REGISTRY")
            .unwrap_or_else(|_| DEFAULT_REGISTRY_URL.to_string()),
        mutation_token: Uuid::new_v4().to_string(),
    };
    let app = router(state);
    let address = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .with_context(|| format!("failed to bind agent007 Hub to {address}"))?;
    let url = format!("http://{address}");

    println!("agent007 Hub is running at {url}");
    println!("Press Ctrl+C to stop it.");
    if open_browser {
        open_url(&url);
    }

    axum::serve(listener, app)
        .await
        .context("agent007 Hub server stopped unexpectedly")
}

fn router(state: HubState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/health", get(api_health))
        .route("/api/projects", get(api_hub))
        .route("/api/hub", get(api_hub))
        .route("/api/packs", get(api_packs))
        .route(
            "/api/packs/{id}",
            get(api_inspect_pack).delete(api_uninstall_pack),
        )
        .route("/api/packs/{id}/install", post(api_install_pack))
        .route("/api/packs/{id}/enable", post(api_enable_pack))
        .route("/api/packs/{id}/disable", post(api_disable_pack))
        .route("/api/packs/{id}/update", post(api_update_pack))
        .route("/api/packs/{id}/rollback", post(api_rollback_pack))
        .route(
            "/api/credentials/{provider}",
            put(api_set_credential).delete(api_delete_credential),
        )
        .route("/api/assets/{kind}/validate", post(api_validate_asset))
        .route("/api/assets/{kind}", post(api_create_asset))
        .route(
            "/api/assets/{kind}/{id}",
            get(api_get_asset)
                .put(api_update_asset)
                .delete(api_delete_asset),
        )
        .with_state(state)
}

async fn api_inspect_pack(
    State(state): State<HubState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<PackInspection>, AssetApiError> {
    hub_pack_manager(&state)?
        .inspect(&id, None, false)
        .await
        .map(Json)
        .map_err(pack_bad_request)
}

fn hub_pack_manager(state: &HubState) -> Result<PackManager, AssetApiError> {
    PackManager::new(
        &state.global_home,
        &state.pack_registry,
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|error| AssetError::Io(error.to_string()).into())
}

async fn api_packs(State(state): State<HubState>) -> Result<Json<HubPackInventory>, AssetApiError> {
    let manager = hub_pack_manager(&state)?;
    let snapshot = manager
        .registry(false)
        .await
        .map_err(|error| AssetError::Io(error.to_string()))?;
    let lock = manager
        .load_lock()
        .map_err(|error| AssetError::Io(error.to_string()))?;
    let mut packs = snapshot
        .index
        .packs
        .into_iter()
        .map(|pack| {
            let installed = lock.packs.get(&pack.id);
            hub_pack_record(pack, installed)
        })
        .collect::<Vec<_>>();
    packs.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(Json(HubPackInventory {
        registry: snapshot.source,
        from_cache: snapshot.from_cache,
        packs,
    }))
}

async fn api_install_pack(
    State(state): State<HubState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(request): Json<PackMutationRequest>,
) -> Result<Json<Value>, AssetApiError> {
    require_mutation_access(&headers, &state)?;
    let manager = hub_pack_manager(&state)?;
    let result = manager
        .install(&id, request.version.as_deref(), true, request.refresh)
        .await
        .map_err(pack_bad_request)?;
    sync_pack_commands(&manager);
    Ok(Json(serde_json::to_value(result).unwrap_or(Value::Null)))
}

async fn api_enable_pack(
    State(state): State<HubState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<LockedPack>, AssetApiError> {
    require_mutation_access(&headers, &state)?;
    let manager = hub_pack_manager(&state)?;
    let pack = manager.enable(&id).map_err(pack_bad_request)?;
    sync_pack_commands(&manager);
    Ok(Json(pack))
}

async fn api_disable_pack(
    State(state): State<HubState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<LockedPack>, AssetApiError> {
    require_mutation_access(&headers, &state)?;
    let manager = hub_pack_manager(&state)?;
    let pack = manager.disable(&id).map_err(pack_bad_request)?;
    sync_pack_commands(&manager);
    Ok(Json(pack))
}

async fn api_update_pack(
    State(state): State<HubState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(request): Json<PackMutationRequest>,
) -> Result<Json<Value>, AssetApiError> {
    require_mutation_access(&headers, &state)?;
    let manager = hub_pack_manager(&state)?;
    let result = manager
        .update(&id, request.refresh)
        .await
        .map_err(pack_bad_request)?;
    sync_pack_commands(&manager);
    Ok(Json(serde_json::to_value(result).unwrap_or(Value::Null)))
}

async fn api_rollback_pack(
    State(state): State<HubState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<LockedPack>, AssetApiError> {
    require_mutation_access(&headers, &state)?;
    let manager = hub_pack_manager(&state)?;
    let pack = manager.rollback(&id).map_err(pack_bad_request)?;
    sync_pack_commands(&manager);
    Ok(Json(pack))
}

async fn api_uninstall_pack(
    State(state): State<HubState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, AssetApiError> {
    require_mutation_access(&headers, &state)?;
    let manager = hub_pack_manager(&state)?;
    manager.uninstall(&id).map_err(pack_bad_request)?;
    sync_pack_commands(&manager);
    Ok(Json(json!({"status": "removed", "id": id})))
}

fn sync_pack_commands(manager: &PackManager) {
    if let Err(error) = super::slash_commands::sync_claude_slash_commands_for_home(manager.home()) {
        tracing::warn!("pack state changed but slash-command sync failed: {error}");
    }
}

fn pack_bad_request(error: anyhow::Error) -> AssetApiError {
    AssetError::BadRequest(error.to_string()).into()
}

fn hub_pack_record(pack: RegistryPack, installed: Option<&LockedPack>) -> HubPack {
    let mut versions = pack
        .versions
        .iter()
        .filter(|version| !version.yanked)
        .filter_map(|version| semver::Version::parse(&version.version).ok())
        .collect::<Vec<_>>();
    versions.sort();
    let latest_version = versions.last().map(ToString::to_string);
    let installed_version = installed.map(|pack| pack.version.clone());
    let update_available = match (&latest_version, &installed_version) {
        (Some(latest), Some(installed)) => match (
            semver::Version::parse(latest),
            semver::Version::parse(installed),
        ) {
            (Ok(latest), Ok(installed)) => latest > installed,
            _ => false,
        },
        _ => false,
    };
    HubPack {
        id: pack.id,
        name: pack.name,
        description: pack.description,
        categories: pack.categories,
        versions: versions
            .into_iter()
            .rev()
            .map(|version| version.to_string())
            .collect(),
        latest_version,
        installed_version,
        enabled: installed.is_some_and(|pack| pack.enabled),
        update_available,
    }
}

async fn api_set_credential(
    State(state): State<HubState>,
    AxumPath(provider): AxumPath<String>,
    headers: HeaderMap,
    Json(request): Json<SetCredentialRequest>,
) -> Result<Json<Value>, AssetApiError> {
    require_mutation_access(&headers, &state)?;
    if credentials::provider(&provider).is_none() {
        return Err(AssetError::BadRequest("unsupported credential provider".to_string()).into());
    }
    if !credentials::keychain_supported() {
        return Err(AssetError::BadRequest(
            "secure credential storage is currently available on macOS only".to_string(),
        )
        .into());
    }
    let key = request.api_key.trim();
    if key.is_empty() {
        return Err(AssetError::BadRequest("API key cannot be empty".to_string()).into());
    }
    if key.len() > 16 * 1024 {
        return Err(AssetError::TooLarge("API key exceeds the 16 KiB limit".to_string()).into());
    }
    credentials::set(&provider, key).map_err(|error| AssetError::Io(error.to_string()))?;
    Ok(Json(json!({
        "status": "saved",
        "provider": provider,
        "configured": true,
        "source": "macOS Keychain",
    })))
}

async fn api_delete_credential(
    State(state): State<HubState>,
    AxumPath(provider): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, AssetApiError> {
    require_mutation_access(&headers, &state)?;
    if credentials::provider(&provider).is_none() {
        return Err(AssetError::BadRequest("unsupported credential provider".to_string()).into());
    }
    if !credentials::keychain_supported() {
        return Err(AssetError::BadRequest(
            "secure credential storage is currently available on macOS only".to_string(),
        )
        .into());
    }
    credentials::delete(&provider).map_err(|error| AssetError::Io(error.to_string()))?;
    Ok(Json(json!({
        "status": "removed",
        "provider": provider,
        "configured": std::env::var_os(credentials::provider(&provider).unwrap().env_var).is_some(),
    })))
}

async fn index(State(state): State<HubState>) -> Html<String> {
    Html(HUB_HTML.replace("__HUB_TOKEN__", &state.mutation_token))
}

async fn api_get_asset(
    State(state): State<HubState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
) -> Result<Json<AssetDocument>, AssetApiError> {
    let kind = AssetKind::parse(&kind)?;
    GlobalAssetStore::new(state.global_home)
        .get_effective(kind, &id)
        .map(Json)
        .map_err(Into::into)
}

async fn api_validate_asset(
    State(state): State<HubState>,
    AxumPath(kind): AxumPath<String>,
    headers: HeaderMap,
    Json(request): Json<ValidateAssetRequest>,
) -> Result<Json<ValidationResult>, AssetApiError> {
    require_mutation_access(&headers, &state)?;
    let kind = AssetKind::parse(&kind)?;
    Ok(Json(GlobalAssetStore::new(state.global_home).validate(
        kind,
        request.format.as_deref(),
        &request.content,
    )))
}

async fn api_create_asset(
    State(state): State<HubState>,
    AxumPath(kind): AxumPath<String>,
    headers: HeaderMap,
    Json(request): Json<CreateAssetRequest>,
) -> Result<(StatusCode, Json<AssetDocument>), AssetApiError> {
    require_mutation_access(&headers, &state)?;
    let kind = AssetKind::parse(&kind)?;
    let document = GlobalAssetStore::new(state.global_home).create(
        kind,
        &request.id,
        request.format.as_deref(),
        &request.content,
    )?;
    Ok((StatusCode::CREATED, Json(document)))
}

async fn api_update_asset(
    State(state): State<HubState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<UpdateAssetRequest>,
) -> Result<Json<AssetDocument>, AssetApiError> {
    require_mutation_access(&headers, &state)?;
    let kind = AssetKind::parse(&kind)?;
    GlobalAssetStore::new(state.global_home)
        .update(
            kind,
            &id,
            &request.expected_revision,
            request.bump,
            &request.content,
        )
        .map(Json)
        .map_err(Into::into)
}

async fn api_delete_asset(
    State(state): State<HubState>,
    AxumPath((kind, id)): AxumPath<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<DeleteAssetRequest>,
) -> Result<Json<Value>, AssetApiError> {
    require_mutation_access(&headers, &state)?;
    let kind = AssetKind::parse(&kind)?;
    GlobalAssetStore::new(state.global_home).delete(
        kind,
        &id,
        &request.expected_revision,
        &request.confirmation,
    )?;
    Ok(Json(json!({"status": "deleted", "id": id})))
}

#[derive(Debug)]
struct AssetApiError(AssetError);

impl From<AssetError> for AssetApiError {
    fn from(error: AssetError) -> Self {
        Self(error)
    }
}

impl IntoResponse for AssetApiError {
    fn into_response(self) -> Response {
        let (status, code, errors) = match self.0 {
            AssetError::BadRequest(message) => {
                (StatusCode::BAD_REQUEST, "bad_request", vec![message])
            }
            AssetError::Forbidden(message) => (StatusCode::FORBIDDEN, "forbidden", vec![message]),
            AssetError::NotFound(message) => (StatusCode::NOT_FOUND, "not_found", vec![message]),
            AssetError::Conflict(message) => {
                (StatusCode::CONFLICT, "revision_conflict", vec![message])
            }
            AssetError::TooLarge(message) => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "asset_too_large",
                vec![message],
            ),
            AssetError::Invalid(errors) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid_asset", errors)
            }
            AssetError::Io(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                vec![message],
            ),
        };
        (
            status,
            Json(json!({
                "error": code,
                "message": errors.first().cloned().unwrap_or_default(),
                "errors": errors,
            })),
        )
            .into_response()
    }
}

fn require_mutation_access(headers: &HeaderMap, state: &HubState) -> Result<(), AssetApiError> {
    let token = headers
        .get("x-agent007-hub-token")
        .and_then(|value| value.to_str().ok());
    if token != Some(state.mutation_token.as_str()) {
        return Err(
            AssetError::Forbidden("missing or invalid Hub mutation token".to_string()).into(),
        );
    }
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !is_loopback_authority(host) {
        return Err(
            AssetError::Forbidden("Hub mutations require a loopback Host".to_string()).into(),
        );
    }
    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        let allowed = reqwest::Url::parse(origin)
            .ok()
            .filter(|url| url.scheme() == "http")
            .and_then(|url| url.host_str().map(str::to_string))
            .map(|host| matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1"))
            .unwrap_or(false);
        if !allowed {
            return Err(AssetError::Forbidden(
                "Hub mutations require a loopback Origin".to_string(),
            )
            .into());
        }
    }
    Ok(())
}

fn is_loopback_authority(authority: &str) -> bool {
    authority == "localhost"
        || authority.starts_with("localhost:")
        || authority == "127.0.0.1"
        || authority.starts_with("127.0.0.1:")
        || authority == "[::1]"
        || authority.starts_with("[::1]:")
}

async fn api_health() -> Json<Value> {
    Json(json!({"status": "ok", "service": "agent007-hub"}))
}

async fn api_hub(
    State(state): State<HubState>,
) -> Result<Json<HubResponse>, (StatusCode, Json<Value>)> {
    build_hub_response(&state).await.map(Json).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error.to_string()})),
        )
    })
}

async fn build_hub_response(state: &HubState) -> Result<HubResponse> {
    let registry = load_registry(&state.registry_path)?;
    let ports = load_ports(&state.ports_path);
    let health_client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(450))
        .build()
        .context("failed to build Hub health client")?;

    let project_futures = project_statuses(&registry).into_iter().map(|status| {
        let configured_port = dashboard_port_for(&status.project.path, &ports);
        let health_client = health_client.clone();
        async move {
            let (threads, threads_error) = load_threads(&status.project.agent_home);
            let thread_count = threads.len();
            let active_thread_count = threads
                .iter()
                .filter(|thread| matches!(thread.status.as_str(), "running" | "awaiting-approval"))
                .count();
            let dashboard_port = live_dashboard_port(&health_client, configured_port).await;
            HubProject {
                status,
                dashboard_port,
                dashboard_url: dashboard_port.map(|port| format!("http://127.0.0.1:{port}")),
                thread_count,
                active_thread_count,
                threads_error,
                threads,
            }
        }
    });
    let projects = futures::future::join_all(project_futures).await;
    let summary = HubSummary {
        projects: projects.len(),
        ready_projects: projects
            .iter()
            .filter(|project| project.status.health.status == "ready")
            .count(),
        running_dashboards: projects
            .iter()
            .filter(|project| project.dashboard_port.is_some())
            .count(),
        threads: projects.iter().map(|project| project.thread_count).sum(),
        active_threads: projects
            .iter()
            .map(|project| project.active_thread_count)
            .sum(),
    };

    Ok(HubResponse {
        generated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        registry_path: state.registry_path.to_string_lossy().to_string(),
        global_home: state.global_home.to_string_lossy().to_string(),
        summary,
        projects,
        assets: load_assets(&state.global_home),
        memory: load_memory_inventory(&state.global_home),
        providers: provider_readiness(&state.global_home),
        capabilities: json!({
            "project_registry": "read-write-cli",
            "threads": "read-only",
            "assets": "read-write-versioned",
            "memory": "inventory-only",
            "provider_settings": if credentials::keychain_supported() { "keychain-read-write" } else { "readiness-only" },
            "process_control": "not-available"
        }),
    })
}

fn load_threads(agent_home: &str) -> (Vec<HubThread>, Option<String>) {
    let sessions_dir = Path::new(agent_home).join("sessions");
    if !sessions_dir.is_dir() {
        return (Vec::new(), None);
    }
    let store = RunStore::new(sessions_dir);
    match store.list_runs(THREAD_LIMIT_PER_PROJECT) {
        Ok(runs) => (
            runs.into_iter()
                .map(|run| HubThread {
                    id: run.id,
                    kind: run.kind,
                    task: run.task,
                    mode: run.mode.clone(),
                    provider: run
                        .provider
                        .filter(|provider| !provider.trim().is_empty())
                        .unwrap_or_else(|| provider_fallback(&run.mode)),
                    started_at: run.started_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                    finished_at: run
                        .finished_at
                        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true)),
                    status: run_status_label(&run.status).to_string(),
                    output_preview: run
                        .output_preview
                        .map(|preview| truncate_text(&preview, 240)),
                })
                .collect(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error.to_string())),
    }
}

fn provider_fallback(mode: &str) -> String {
    match mode {
        "hosted-mcp" => "Host LLM".to_string(),
        "standalone" => "Standalone".to_string(),
        other if !other.trim().is_empty() => other.to_string(),
        _ => "Unknown".to_string(),
    }
}

fn run_status_label(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::AwaitingApproval => "awaiting-approval",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
    }
}

fn truncate_text(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

async fn live_dashboard_port(client: &reqwest::Client, port: Option<u16>) -> Option<u16> {
    let port = port?;
    let response = client
        .get(format!("http://127.0.0.1:{port}/api/health"))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let payload = response.json::<Value>().await.ok()?;
    (payload.get("status").and_then(Value::as_str) == Some("ok")
        && payload.get("version").and_then(Value::as_str).is_some())
    .then_some(port)
}

fn load_ports(path: &Path) -> HashMap<String, u16> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| toml::from_str::<PortRegistryFile>(&content).ok())
        .map(|registry| registry.projects)
        .unwrap_or_default()
}

fn dashboard_port_for(path: &str, ports: &HashMap<String, u16>) -> Option<u16> {
    ports.get(path).copied().or_else(|| {
        Path::new(path)
            .canonicalize()
            .ok()
            .and_then(|canonical| ports.get(&canonical.to_string_lossy().to_string()).copied())
    })
}

fn load_assets(global_home: &Path) -> AssetInventory {
    let store = GlobalAssetStore::new(global_home);
    AssetInventory {
        skills: store.list_effective(AssetKind::Skill).unwrap_or_default(),
        workflows: store
            .list_effective(AssetKind::Workflow)
            .unwrap_or_default(),
        personas: store.list_effective(AssetKind::Persona).unwrap_or_default(),
    }
}

fn load_memory_inventory(global_home: &Path) -> MemoryInventory {
    let memory_path = global_home.join("memory");
    let database_path = memory_path.join("memory.db");
    let mut storage_groups = std::fs::read_dir(&memory_path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .map(|value| value.is_dir())
                .unwrap_or(false)
        })
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    storage_groups.sort();
    storage_groups.truncate(24);
    MemoryInventory {
        path: memory_path.to_string_lossy().to_string(),
        database_present: database_path.is_file(),
        database_size_bytes: database_path
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or_default(),
        storage_groups,
        management_mode: "inventory-only",
    }
}

fn provider_readiness(global_home: &Path) -> Vec<ProviderReadiness> {
    let config_path = global_home.join("config.toml");
    let config = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|raw| toml::from_str::<toml::Value>(&raw).ok());
    let openai_env = std::env::var_os("OPENAI_API_KEY").is_some();
    let anthropic_env = std::env::var_os("ANTHROPIC_API_KEY").is_some();
    let openai_keychain = credentials::keychain_contains("openai");
    let anthropic_keychain = credentials::keychain_contains("anthropic");
    let ollama_env = std::env::var("OLLAMA_HOST").ok();
    let openai_config = config_has_table(&config, &["models", "codex"]);
    let anthropic_config = config_has_table(&config, &["models", "claude"]);
    let ollama_config = config_has_table(&config, &["models", "ollama"]);
    let ollama_endpoint = ollama_env
        .as_deref()
        .map(redact_url)
        .or_else(|| {
            config_string(&config, &["models", "ollama", "base_url"])
                .map(|value| redact_url(&value))
        })
        .unwrap_or_else(|| "http://127.0.0.1:11434".to_string());

    vec![
        ProviderReadiness {
            id: "hosted-mcp",
            name: "Hosted MCP",
            category: "host",
            configured: true,
            credential_editable: false,
            credential_managed: false,
            source: "editor host".to_string(),
            detail: "Uses the connected Claude, Codex, Cursor, or other MCP host.".to_string(),
        },
        ProviderReadiness {
            id: "anthropic",
            name: "Anthropic",
            category: "cloud",
            configured: anthropic_env || anthropic_config || anthropic_keychain,
            credential_editable: credentials::keychain_supported(),
            credential_managed: anthropic_keychain,
            source: readiness_source(anthropic_env, anthropic_config, anthropic_keychain),
            detail: "Set or rotate this key without exposing its stored value.".to_string(),
        },
        ProviderReadiness {
            id: "openai",
            name: "OpenAI",
            category: "cloud",
            configured: openai_env || openai_config || openai_keychain,
            credential_editable: credentials::keychain_supported(),
            credential_managed: openai_keychain,
            source: readiness_source(openai_env, openai_config, openai_keychain),
            detail: "Set or rotate this key without exposing its stored value.".to_string(),
        },
        ProviderReadiness {
            id: "ollama",
            name: "Ollama",
            category: "local",
            configured: ollama_env.is_some() || ollama_config,
            credential_editable: false,
            credential_managed: false,
            source: readiness_source(ollama_env.is_some(), ollama_config, false),
            detail: format!("Endpoint: {ollama_endpoint}"),
        },
    ]
}

fn config_has_table(config: &Option<toml::Value>, path: &[&str]) -> bool {
    config_value(config, path)
        .and_then(toml::Value::as_table)
        .is_some()
}

fn config_string(config: &Option<toml::Value>, path: &[&str]) -> Option<String> {
    config_value(config, path)
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned)
}

fn config_value<'a>(config: &'a Option<toml::Value>, path: &[&str]) -> Option<&'a toml::Value> {
    let mut value = config.as_ref()?;
    for segment in path {
        value = value.get(*segment)?;
    }
    Some(value)
}

fn readiness_source(env: bool, config: bool, keychain: bool) -> String {
    let mut sources = Vec::new();
    if env {
        sources.push("environment");
    }
    if config {
        sources.push("config");
    }
    if keychain {
        sources.push("macOS Keychain");
    }
    if sources.is_empty() {
        "not configured".to_string()
    } else {
        sources.join(" + ")
    }
}

fn redact_url(value: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(value) else {
        return "[invalid endpoint]".to_string();
    };
    if !url.username().is_empty() {
        let _ = url.set_username("redacted");
    }
    if url.password().is_some() {
        let _ = url.set_password(Some("redacted"));
    }
    url.to_string().trim_end_matches('/').to_string()
}

#[cfg(target_os = "macos")]
fn open_url(url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_url(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

#[cfg(windows)]
fn open_url(url: &str) {
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
}

#[cfg(not(any(unix, windows)))]
fn open_url(_url: &str) {}

const HUB_HTML: &str = include_str!("hub.html");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::projects::{ProjectEntry, ProjectRegistry};

    fn entry(name: &str, path: &Path) -> ProjectEntry {
        ProjectEntry {
            id: format!("proj-{name}"),
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            agent_home: path.join(".agent007").to_string_lossy().to_string(),
            added_at: "2026-06-17T10:00:00Z".to_string(),
            last_seen_at: "2026-06-17T10:00:00Z".to_string(),
        }
    }

    async fn spawn_dashboard_health() -> (u16, tokio::task::JoinHandle<()>) {
        let app = Router::new().route(
            "/api/health",
            get(|| async { Json(json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")})) }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (port, handle)
    }

    #[tokio::test]
    async fn response_summarizes_threads_assets_and_live_dashboard() {
        let temp = tempfile::tempdir().unwrap();
        let ready = temp.path().join("ready");
        let global_home = temp.path().join("global");
        std::fs::create_dir_all(ready.join(".agent007/sessions")).unwrap();
        std::fs::write(ready.join(".agent007/config.toml"), "").unwrap();
        std::fs::create_dir_all(global_home.join("skills")).unwrap();
        std::fs::write(
            global_home.join("skills/demo.md"),
            "---\nname: Demo\ndescription: Example skill\ntrigger: /demo\nversion: \"1.0.0\"\n---\n\nDo demo work.\n",
        )
        .unwrap();
        let store = RunStore::new(ready.join(".agent007/sessions"));
        store
            .create_run("task", "Review the Hub", "hosted-mcp", Some("codex"))
            .unwrap();

        let registry_path = temp.path().join("projects.json");
        std::fs::write(
            &registry_path,
            serde_json::to_vec(&ProjectRegistry {
                version: 1,
                projects: vec![entry("ready", &ready)],
            })
            .unwrap(),
        )
        .unwrap();
        let (dashboard_port, handle) = spawn_dashboard_health().await;
        let ports_path = temp.path().join("ports.toml");
        std::fs::write(
            &ports_path,
            format!(
                "[projects]\n\"{}\" = {dashboard_port}\n",
                ready.to_string_lossy()
            ),
        )
        .unwrap();

        let response = build_hub_response(&HubState {
            registry_path,
            ports_path,
            global_home,
            pack_registry: DEFAULT_REGISTRY_URL.to_string(),
            mutation_token: "test-token".to_string(),
        })
        .await
        .unwrap();
        handle.abort();

        assert_eq!(response.summary.projects, 1);
        assert_eq!(response.summary.threads, 1);
        assert_eq!(response.summary.active_threads, 1);
        assert_eq!(response.summary.running_dashboards, 1);
        assert_eq!(response.projects[0].threads[0].provider, "codex");
        assert_eq!(response.assets.skills[0].name, "Demo");
    }

    #[tokio::test]
    async fn unrelated_service_is_not_reported_as_dashboard() {
        let app = Router::new().route(
            "/api/health",
            get(|| async { Json(json!({"status": "ok", "service": "other"})) }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        assert_eq!(live_dashboard_port(&client, Some(port)).await, None);
        handle.abort();
    }

    #[test]
    fn provider_readiness_never_contains_key_values() {
        let temp = tempfile::tempdir().unwrap();
        let providers = provider_readiness(temp.path());
        let json = serde_json::to_string(&providers).unwrap();
        assert!(!json.contains("sk-"));
        assert!(!json.contains("API_KEY"));
    }

    #[test]
    fn provider_readiness_recognizes_runtime_config_tables() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("config.toml"),
            "[models.claude]\ndefault_model = \"claude-sonnet\"\n\n[models.codex]\ndefault_model = \"gpt-codex\"\n",
        )
        .unwrap();

        let providers = provider_readiness(temp.path());
        for id in ["anthropic", "openai"] {
            let provider = providers.iter().find(|provider| provider.id == id).unwrap();
            assert!(provider.configured, "{id} should be configured");
            assert!(provider.source.contains("config"));
        }
    }

    #[test]
    fn malformed_provider_endpoint_is_not_reflected() {
        assert_eq!(
            redact_url("not-a-url?token=must-not-be-reflected"),
            "[invalid endpoint]"
        );
    }

    #[test]
    fn hub_html_contains_project_tree_and_global_surfaces() {
        assert!(HUB_HTML.contains("agent007 Hub"));
        assert!(HUB_HTML.contains("project-tree"));
        assert!(HUB_HTML.contains("data-action=\"projects-root\""));
        assert!(HUB_HTML.contains("class=\"thread-group-heading\""));
        assert!(HUB_HTML.contains("aria-level=\"3\""));
        assert!(HUB_HTML.contains("treeInitialized"));
        assert!(HUB_HTML.contains("data-view=\"skills\""));
        assert!(HUB_HTML.contains("data-view=\"settings\""));
        assert!(HUB_HTML.contains("data-view=\"packs\""));
        assert!(HUB_HTML.contains("/api/packs/"));
        assert!(HUB_HTML.contains("id=\"pack-search\""));
        assert!(HUB_HTML.contains("global scope"));
        assert!(HUB_HTML.contains("Verified before activation"));
        assert!(HUB_HTML.contains("api('/api/hub'"));
        assert!(HUB_HTML.contains("/api/credentials/"));
        assert!(HUB_HTML.contains("type=\"password\""));
        assert!(HUB_HTML.contains("autocomplete=\"new-password\""));
        assert!(HUB_HTML.contains("data-credential-remove"));
        assert!(HUB_HTML.contains("credentialDirty"));
        assert!(HUB_HTML.contains("__HUB_TOKEN__"));
        for theme in ["night", "forest", "ocean", "aurora", "day", "corporate"] {
            assert!(HUB_HTML.contains(&format!("[data-theme=\"{theme}\"]")));
        }
        assert!(HUB_HTML.contains("id=\"asset-source\""));
        assert!(HUB_HTML.contains("id=\"asset-bump\""));
        assert!(HUB_HTML.contains("class=\"asset-copy\""));
        assert!(HUB_HTML.contains("class=\"asset-title-line\""));
        assert!(HUB_HTML.contains("class=\"asset-row-meta\""));
        assert!(HUB_HTML.contains("X-Agent007-Hub-Token"));
    }

    #[test]
    fn hub_pack_record_uses_semver_and_local_state() {
        let pack = RegistryPack {
            id: "example".to_string(),
            name: "Example".to_string(),
            description: "Example pack".to_string(),
            categories: vec!["testing".to_string()],
            tags: vec![],
            versions: vec![
                agent007_packs::RegistryPackVersion {
                    version: "1.9.0".to_string(),
                    min_agent007: "0.6.0".to_string(),
                    manifest_url: "manifest".to_string(),
                    manifest_sha256: "a".repeat(64),
                    artifact_url: "artifact".to_string(),
                    artifact_sha256: "b".repeat(64),
                    size_bytes: 1,
                    published_at: "2026-06-18T00:00:00Z".to_string(),
                    yanked: false,
                },
                agent007_packs::RegistryPackVersion {
                    version: "1.10.0".to_string(),
                    min_agent007: "0.6.0".to_string(),
                    manifest_url: "manifest".to_string(),
                    manifest_sha256: "a".repeat(64),
                    artifact_url: "artifact".to_string(),
                    artifact_sha256: "b".repeat(64),
                    size_bytes: 1,
                    published_at: "2026-06-18T00:00:00Z".to_string(),
                    yanked: false,
                },
            ],
        };
        let installed = LockedPack {
            id: "example".to_string(),
            version: "1.9.0".to_string(),
            enabled: true,
            installed_at: "2026-06-18T00:00:00Z".to_string(),
            registry: "fixture".to_string(),
            artifact_sha256: "b".repeat(64),
            manifest_sha256: "a".repeat(64),
            history: vec![],
        };
        let record = hub_pack_record(pack, Some(&installed));
        assert_eq!(record.latest_version.as_deref(), Some("1.10.0"));
        assert!(record.enabled);
        assert!(record.update_available);
    }

    #[test]
    fn mutation_guard_requires_loopback_host_and_matching_token() {
        let state = HubState {
            registry_path: PathBuf::new(),
            ports_path: PathBuf::new(),
            global_home: PathBuf::new(),
            pack_registry: DEFAULT_REGISTRY_URL.to_string(),
            mutation_token: "secret".to_string(),
        };
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:8006".parse().unwrap());
        headers.insert("x-agent007-hub-token", "secret".parse().unwrap());
        headers.insert(header::ORIGIN, "http://localhost:8006".parse().unwrap());
        assert!(require_mutation_access(&headers, &state).is_ok());

        headers.insert("x-agent007-hub-token", "wrong".parse().unwrap());
        assert!(require_mutation_access(&headers, &state).is_err());
        headers.insert("x-agent007-hub-token", "secret".parse().unwrap());
        headers.insert(header::ORIGIN, "https://attacker.example".parse().unwrap());
        assert!(require_mutation_access(&headers, &state).is_err());
    }
}
