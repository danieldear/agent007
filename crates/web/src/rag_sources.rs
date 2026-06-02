use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent007_memory::vectordb::LanceDBStore;
use agent007_memory::{Indexer, Retriever};
use agent007_models::{
    EmbeddingProvider, LocalHashEmbeddingProvider, MockProvider, OllamaEmbeddingProvider,
};
use serde::{Deserialize, Serialize};

// ── data model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RagKind {
    Url,
    File,
    Directory,
}

impl RagKind {
    pub fn as_str(&self) -> &str {
        match self {
            RagKind::Url => "url",
            RagKind::File => "file",
            RagKind::Directory => "directory",
        }
    }
}

impl std::fmt::Display for RagKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagSource {
    pub id: String,
    pub name: String,
    pub kind: RagKind,
    pub source_ref: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<usize>,
    #[serde(default = "default_status")]
    pub status: String, // "pending" | "indexing" | "ready" | "error"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_msg: Option<String>,
    pub added_at: String,
}

fn default_scope() -> String {
    "project".to_string()
}

fn default_chunk_size() -> usize {
    512
}

fn default_status() -> String {
    "pending".to_string()
}

// ── storage ───────────────────────────────────────────────────────────────────

fn sources_path(project_home: &Path) -> PathBuf {
    project_home.join("rag").join("sources.json")
}

pub fn load_rag_sources(project_home: &Path) -> Result<Vec<RagSource>, String> {
    let path = sources_path(project_home);
    if !path.exists() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_json::from_str::<Vec<RagSource>>(&raw)
        .map_err(|e| format!("invalid {}: {e}", path.display()))
}

pub fn save_rag_sources(project_home: &Path, sources: &[RagSource]) -> Result<(), String> {
    let path = sources_path(project_home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(sources).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

// ── operations ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddRagSourceRequest {
    pub name: String,
    pub kind: RagKind,
    pub source_ref: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
}

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn add_rag_source(project_home: &Path, req: AddRagSourceRequest) -> Result<RagSource, String> {
    let mut sources = load_rag_sources(project_home)?;
    let id = slugify(&req.name);

    if sources.iter().any(|s| s.id == id) {
        return Err(format!("RAG source '{}' already exists", id));
    }

    let source = RagSource {
        id,
        name: req.name,
        kind: req.kind,
        source_ref: req.source_ref,
        scope: req.scope,
        chunk_size: req.chunk_size,
        indexed_at: None,
        chunk_count: None,
        status: "pending".to_string(),
        error_msg: None,
        added_at: chrono::Utc::now().to_rfc3339(),
    };

    sources.push(source.clone());
    save_rag_sources(project_home, &sources)?;
    Ok(source)
}

pub fn delete_rag_source(project_home: &Path, id: &str) -> Result<(), String> {
    let mut sources = load_rag_sources(project_home)?;
    let before = sources.len();
    sources.retain(|s| s.id != id);
    if sources.len() == before {
        return Err(format!("RAG source '{}' not found", id));
    }
    save_rag_sources(project_home, &sources)
}

/// Mark source as indexing, attempt to read text content, update status.
/// Indexing into LanceDB requires the full model/embedding setup — here we
/// perform a lightweight "file can be read" check and mark ready.
pub async fn reindex_rag_source(project_home: &Path, id: &str) -> Result<RagSource, String> {
    // Set status → indexing
    {
        let mut sources = load_rag_sources(project_home)?;
        if let Some(s) = sources.iter_mut().find(|s| s.id == id) {
            s.status = "indexing".to_string();
            s.error_msg = None;
        }
        save_rag_sources(project_home, &sources)?;
    }

    let sources = load_rag_sources(project_home)?;
    let source = sources
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("RAG source '{}' not found", id))?
        .clone();

    let (chunk_count, error) = index_source_content(project_home, &source).await;

    let mut sources = load_rag_sources(project_home)?;
    let target = sources
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("RAG source '{}' not found after index", id))?;

    if let Some(err) = error {
        target.status = "error".to_string();
        target.error_msg = Some(err);
    } else {
        target.status = "ready".to_string();
        target.error_msg = None;
        target.indexed_at = Some(chrono::Utc::now().to_rfc3339());
        target.chunk_count = chunk_count;
    }

    let result = target.clone();
    save_rag_sources(project_home, &sources)?;
    Ok(result)
}

async fn index_source_content(
    project_home: &Path,
    source: &RagSource,
) -> (Option<usize>, Option<String>) {
    let home = project_home;
    let embedder = build_embedder(project_home);
    let health_embedding = match embedder.embed("agent007-rag-health").await {
        Ok(v) => v,
        Err(e) => {
            return (
                None,
                Some(format!(
                    "embedding provider health check failed before RAG content indexing: {}",
                    friendly_embedding_error(&e.to_string())
                )),
            );
        }
    };
    let is_mock = health_embedding.iter().all(|x| *x == 0.0);
    let embed_dim = if is_mock {
        384
    } else {
        match embedder.embed("agent007-rag-dim").await {
            Ok(v) => v.len().max(1),
            Err(e) => {
                return (
                    None,
                    Some(format!(
                        "embedding provider dimension probe failed before RAG content indexing: {}",
                        friendly_embedding_error(&e.to_string())
                    )),
                );
            }
        }
    };
    let db_dir = home.join("vectordb");
    let db_path = db_dir.to_string_lossy().to_string();
    let db = match LanceDBStore::new(&db_path, "rag_sources", embed_dim).await {
        Ok(db) => Arc::new(db) as Arc<dyn agent007_memory::VectorDB>,
        Err(e) => return (None, Some(format!("failed to initialize LanceDB: {e}"))),
    };
    let indexer = Indexer::new(embedder, db, source.chunk_size.max(64));

    match gather_source_text(source).await {
        Ok((content, approx_chunks)) => {
            if content.trim().is_empty() {
                return (Some(0), None);
            }
            let doc_id = format!("rag:{}:{}", source.kind, source.id);
            match indexer.index_text(&doc_id, &content).await {
                Ok(_) => (Some(approx_chunks), None),
                Err(e) => (
                    None,
                    Some(format!(
                        "failed to index RAG content after reading {} source '{}': {}",
                        source.kind,
                        source.source_ref,
                        friendly_embedding_error(&e.to_string())
                    )),
                ),
            }
        }
        Err(e) => (None, Some(e)),
    }
}

fn friendly_embedding_error(message: &str) -> String {
    if message.contains("ollama-embed") && message.contains("HTTP 404") {
        let model = std::env::var("AGENT007_OLLAMA_EMBED_MODEL")
            .unwrap_or_else(|_| "nomic-embed-text".to_string());
        return format!(
            "Ollama embedding request returned HTTP 404 for model '{model}'. This is an embedding setup problem, not a URL-vs-folder RAG source problem. If you do not want Ollama, set AGENT007_RAG_EMBED_PROVIDER=local or remove memory.rag.embedding_provider=ollama from config.toml. If you do want Ollama, run `ollama pull {model}` or set AGENT007_RAG_EMBED_MODEL to an installed embedding model. Details: {message}"
        );
    }

    if message.contains("ollama-embed") {
        return format!(
            "Ollama embedding request failed. If you do not want Ollama, set AGENT007_RAG_EMBED_PROVIDER=local or remove memory.rag.embedding_provider=ollama from config.toml. Otherwise check AGENT007_OLLAMA_BASE_URL / AGENT007_RAG_EMBED_MODEL. Details: {message}"
        );
    }

    message.to_string()
}

#[derive(Debug, Default, serde::Deserialize)]
struct RagEmbeddingFileConfig {
    memory: Option<RagMemorySection>,
    models: Option<RagModelsSection>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RagMemorySection {
    rag: Option<RagMemoryConfig>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RagMemoryConfig {
    embedding_provider: Option<String>,
    embedding_model: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RagModelsSection {
    ollama: Option<RagOllamaConfig>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RagOllamaConfig {
    base_url: Option<String>,
}

fn build_embedder(project_home: &Path) -> Arc<dyn EmbeddingProvider> {
    if std::env::var("AGENT007_FORCE_MOCK_EMBED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return Arc::new(MockProvider::with_embedding_dim("", "mock-embed", 384))
            as Arc<dyn EmbeddingProvider>;
    }

    let file_cfg = read_rag_embedding_file_config(project_home);
    let provider = std::env::var("AGENT007_RAG_EMBED_PROVIDER")
        .ok()
        .or_else(|| std::env::var("AGENT007_EMBEDDING_PROVIDER").ok())
        .or_else(|| {
            file_cfg
                .as_ref()
                .and_then(|cfg| cfg.memory.as_ref())
                .and_then(|memory| memory.rag.as_ref())
                .and_then(|rag| rag.embedding_provider.clone())
        })
        .unwrap_or_else(|| "local".to_string())
        .to_lowercase();

    match provider.as_str() {
        "ollama" => {
            let base_url = std::env::var("AGENT007_OLLAMA_BASE_URL")
                .ok()
                .or_else(|| {
                    file_cfg
                        .as_ref()
                        .and_then(|cfg| cfg.models.as_ref())
                        .and_then(|models| models.ollama.as_ref())
                        .and_then(|ollama| ollama.base_url.clone())
                })
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            let model = std::env::var("AGENT007_RAG_EMBED_MODEL")
                .ok()
                .or_else(|| std::env::var("AGENT007_OLLAMA_EMBED_MODEL").ok())
                .or_else(|| {
                    file_cfg
                        .as_ref()
                        .and_then(|cfg| cfg.memory.as_ref())
                        .and_then(|memory| memory.rag.as_ref())
                        .and_then(|rag| rag.embedding_model.clone())
                })
                .unwrap_or_else(|| "nomic-embed-text".to_string());
            Arc::new(OllamaEmbeddingProvider::new(&base_url, &model)) as Arc<dyn EmbeddingProvider>
        }
        "mock" | "none" | "zero" => {
            Arc::new(MockProvider::with_embedding_dim("", "mock-embed", 384))
                as Arc<dyn EmbeddingProvider>
        }
        // Default: no external service. This avoids forcing Ollama while still
        // producing non-zero lexical vectors for local folder/file RAG.
        _ => Arc::new(LocalHashEmbeddingProvider::new(384)) as Arc<dyn EmbeddingProvider>,
    }
}

fn read_rag_embedding_file_config(project_home: &Path) -> Option<RagEmbeddingFileConfig> {
    let path = std::env::var("AGENT007_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| project_home.join("config.toml"));
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

async fn gather_source_text(source: &RagSource) -> Result<(String, usize), String> {
    match source.kind {
        RagKind::File => match tokio::fs::read_to_string(&source.source_ref).await {
            Ok(content) => {
                let chunks = content.len().div_ceil(source.chunk_size.max(64));
                Ok((content, chunks))
            }
            Err(e) => Err(format!("cannot read file: {}", e)),
        },
        RagKind::Directory => match std::fs::read_dir(&source.source_ref) {
            Ok(entries) => {
                let mut merged = String::new();
                let mut files = 0usize;
                for entry in entries.flatten().filter(|e| e.path().is_file()) {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        files += 1;
                        merged.push_str("\n\n");
                        merged.push_str(&content);
                    }
                }
                Ok((merged, files))
            }
            Err(e) => Err(format!("cannot read directory: {}", e)),
        },
        RagKind::Url => {
            if source.source_ref.starts_with("http://") || source.source_ref.starts_with("https://")
            {
                let response = reqwest::get(&source.source_ref)
                    .await
                    .map_err(|e| format!("failed to fetch URL: {e}"))?;
                let text = response
                    .text()
                    .await
                    .map_err(|e| format!("failed to read URL body: {e}"))?;
                let chunks = text.len().div_ceil(source.chunk_size.max(64));
                Ok((text, chunks.max(1)))
            } else {
                Err("source_ref must be a valid http/https URL".to_string())
            }
        }
    }
}

pub async fn query_rag_sources(
    project_home: &Path,
    query: &str,
    limit: usize,
) -> Result<(String, serde_json::Value), String> {
    let sources = load_rag_sources(project_home)?;
    let ready = sources.iter().filter(|s| s.status == "ready").count();
    let embedder = build_embedder(project_home);
    let probe = embedder
        .embed("agent007-rag-health")
        .await
        .map_err(|e| format!("embedding provider error: {e}"))?;
    let mock_embedding = probe.iter().all(|x| *x == 0.0);
    let embed_dim = probe.len().max(1);

    let db_dir = project_home.join("vectordb");
    let db_path = db_dir.to_string_lossy().to_string();
    let db = LanceDBStore::new(&db_path, "rag_sources", embed_dim)
        .await
        .map_err(|e| format!("failed to initialize LanceDB: {e}"))?;
    let retriever = Retriever::new(embedder, Arc::new(db), limit.max(1));
    let (context, stats) = retriever
        .retrieve_with_stats(query)
        .await
        .map_err(|e| format!("retrieval failed: {e}"))?;
    let meta = serde_json::json!({
        "mode": "vector-search",
        "query": query,
        "ready_sources": ready,
        "stats": stats,
        "mock_embedding": mock_embedding,
    });
    Ok((context, meta))
}

#[cfg(test)]
mod tests {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    use super::{
        add_rag_source, friendly_embedding_error, load_rag_sources, AddRagSourceRequest, RagKind,
    };

    #[test]
    fn load_sources_reports_invalid_json() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("rag");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sources.json"), "not-json").unwrap();
        let err = load_rag_sources(tmp.path()).unwrap_err();
        assert!(err.contains("invalid"));
    }

    #[test]
    fn default_embedder_is_local_not_ollama() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("AGENT007_FORCE_MOCK_EMBED");
        std::env::remove_var("AGENT007_RAG_EMBED_PROVIDER");
        std::env::remove_var("AGENT007_EMBEDDING_PROVIDER");
        std::env::remove_var("AGENT007_CONFIG");
        let tmp = tempfile::tempdir().unwrap();
        let embedder = super::build_embedder(tmp.path());
        assert_eq!(embedder.name(), "local-hash-embed");
    }

    #[test]
    fn config_can_opt_into_ollama_embedder() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("AGENT007_FORCE_MOCK_EMBED");
        std::env::remove_var("AGENT007_RAG_EMBED_PROVIDER");
        std::env::remove_var("AGENT007_EMBEDDING_PROVIDER");
        std::env::remove_var("AGENT007_CONFIG");
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "[memory.rag]\nembedding_provider = \"ollama\"\nembedding_model = \"nomic-embed-text\"\n",
        )
        .unwrap();
        let embedder = super::build_embedder(tmp.path());
        assert_eq!(embedder.name(), "ollama-embed");
    }

    #[test]
    fn friendly_embedding_error_explains_ollama_404() {
        let msg = friendly_embedding_error(
            "Embedding error: api error from ollama-embed: HTTP 404 Not Found: model missing",
        );
        assert!(msg.contains("embedding setup problem"));
        assert!(msg.contains("not a URL-vs-folder RAG source problem"));
        assert!(msg.contains("ollama pull"));
    }

    #[test]
    fn add_source_starts_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let source = add_rag_source(
            tmp.path(),
            AddRagSourceRequest {
                name: "Docs".to_string(),
                kind: RagKind::Url,
                source_ref: "https://example.com/docs".to_string(),
                scope: "project".to_string(),
                chunk_size: 512,
            },
        )
        .unwrap();
        assert_eq!(source.status, "pending");
    }
}
