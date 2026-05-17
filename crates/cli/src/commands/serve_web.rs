use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;

use crate::commands::run::{
    agent007_global_home, build_stack_for_web, provider_readiness_response, runtime_mode_label,
    selected_runtime_model, selected_runtime_provider, standalone_mode_available,
};
use crate::config::Config;
use agent007_web::WebServer;

pub const DEFAULT_PORT: u16 = 8007;

pub async fn execute(config: Arc<Config>, port: u16) -> Result<()> {
    tracing::info!("building agent007 stack for web server…");

    let stack = build_stack_for_web(&config).await?;

    // Spawn feedback collector in background (same as the run command).
    let collector = stack.feedback_collector.clone();
    stack.tracker.spawn(async move {
        if let Err(e) = collector.run().await {
            tracing::warn!("feedback collector error: {e}");
        }
    });

    let cwd = std::env::current_dir().unwrap_or_default();
    let registry = PortRegistry::load();
    let actual_port = registry.resolve_port(&cwd, port).await;
    PortRegistry::register(&cwd, actual_port);

    tracing::info!("agent007 web dashboard starting on http://0.0.0.0:{actual_port}");
    let standalone_mode = standalone_mode_available(&config);
    let runtime_mode = runtime_mode_label(&config).to_string();
    let provider_label = match (
        selected_runtime_provider(&config),
        selected_runtime_model(&config),
    ) {
        (Some(provider), Some(model)) if provider != model => format!("{provider} / {model}"),
        (Some(provider), _) => provider,
        _ => "hosted-mcp".to_string(),
    };

    let provider_readiness = provider_readiness_response(&config);
    let web = WebServer::new_with_provider_readiness(
        stack.dispatcher.clone(),
        stack.learning_dispatcher.clone(),
        stack.model_router.clone(),
        Some(stack.workflow_runner.clone()),
        stack.cancel.clone(),
        standalone_mode,
        runtime_mode,
        provider_label,
        provider_readiness,
    );

    tracing::info!("starting axum serve loop");
    let result = web
        .run(actual_port)
        .await
        .map_err(|e| anyhow::anyhow!("web server error: {e}"));
    if let Err(ref e) = result {
        tracing::error!("web server stopped with error: {e}");
    }

    // Clean up registry entry when server stops
    PortRegistry::unregister(&cwd);

    stack.tracker.close();
    stack.tracker.wait().await;
    result
}

/// Global port registry stored at ~/.agent007/ports.toml.
/// Maps project directory → assigned port so multiple projects can
/// coexist without port collisions.
struct PortRegistry {
    entries: HashMap<String, u16>,
}

impl PortRegistry {
    fn registry_path() -> std::path::PathBuf {
        agent007_global_home().join("ports.toml")
    }

    fn load() -> Self {
        let path = Self::registry_path();
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str::<HashMap<String, HashMap<String, u16>>>(&s).ok())
            .and_then(|t| t.get("projects").cloned())
            .unwrap_or_default();
        Self { entries }
    }

    fn save(entries: &HashMap<String, u16>) {
        let path = Self::registry_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let table = toml::Value::Table({
            let mut root = toml::map::Map::new();
            let mut projects = toml::map::Map::new();
            for (k, v) in entries {
                projects.insert(k.clone(), toml::Value::Integer(*v as i64));
            }
            root.insert("projects".to_string(), toml::Value::Table(projects));
            root
        });
        let _ = std::fs::write(path, toml::to_string_pretty(&table).unwrap_or_default());
    }

    /// Resolve the port to use for the given project directory:
    /// 1. If a port is registered for this project and it's free → reuse it
    /// 2. If a port is registered but occupied by something else → reuse it anyway
    ///    (it's ours — the old process may have just died)
    /// 3. If not registered → find a free port not used by any other project
    async fn resolve_port(&self, cwd: &std::path::Path, preferred: u16) -> u16 {
        let key = project_registry_key(cwd);
        let used_ports: std::collections::HashSet<u16> = self.entries.values().copied().collect();

        if let Some(&registered) = self.entries.get(&key) {
            return registered;
        }

        // Find a free port not already assigned to another project
        for offset in 0u16..50 {
            let port = preferred.wrapping_add(offset);
            if used_ports.contains(&port) {
                continue;
            }
            let addr = format!("0.0.0.0:{port}");
            if let Ok(listener) = tokio::net::TcpListener::bind(&addr).await {
                drop(listener);
                return port;
            }
        }
        preferred
    }

    /// Write (or update) the registry entry for this project.
    fn register(cwd: &std::path::Path, port: u16) {
        let mut reg = Self::load();
        reg.entries.insert(project_registry_key(cwd), port);
        Self::save(&reg.entries);
    }

    /// Remove the registry entry when the dashboard exits.
    fn unregister(cwd: &std::path::Path) {
        let mut reg = Self::load();
        reg.entries.remove(&project_registry_key(cwd));
        Self::save(&reg.entries);
    }
}

fn project_registry_key(cwd: &Path) -> String {
    let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    discover_project_root(&canonical)
        .unwrap_or(canonical)
        .to_string_lossy()
        .to_string()
}

fn discover_project_root(from: &Path) -> Option<PathBuf> {
    let mut dir = from.to_path_buf();
    loop {
        if dir.join(".agent007").is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_port_is_8007() {
        assert_eq!(DEFAULT_PORT, 8007u16);
    }

    #[tokio::test]
    async fn resolve_port_returns_registered_port() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = PortRegistry {
            entries: HashMap::new(),
        };
        registry
            .entries
            .insert(project_registry_key(dir.path()), 9001);
        let port = registry.resolve_port(dir.path(), 8007).await;
        assert_eq!(port, 9001);
    }

    #[test]
    fn project_registry_key_collapses_nested_paths_to_project_root() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".agent007")).unwrap();
        let nested = root.path().join("src").join("module");
        std::fs::create_dir_all(&nested).unwrap();
        let expected = root.path().canonicalize().unwrap();
        assert_eq!(
            project_registry_key(&nested),
            expected.to_string_lossy().to_string()
        );
    }

    #[tokio::test]
    async fn resolve_port_skips_ports_used_by_other_projects() {
        let dir = tempfile::tempdir().unwrap();
        // Bind 8007 so it's occupied
        let _listener = tokio::net::TcpListener::bind("0.0.0.0:19999")
            .await
            .unwrap();
        let mut registry = PortRegistry {
            entries: HashMap::new(),
        };
        // Mark 19999 as used by another project
        registry.entries.insert("/other/project".to_string(), 19999);
        let port = registry.resolve_port(dir.path(), 19999).await;
        // Should skip 19999 (used by another project) and find a different port
        assert_ne!(port, 19999);
    }
}
