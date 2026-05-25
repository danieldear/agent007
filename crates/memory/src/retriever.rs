use crate::error::MemoryError;
use crate::store::MemoryStore;
use crate::vectordb::VectorDB;
use agent007_models::EmbeddingProvider;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetrieveStats {
    pub query_chars: usize,
    pub graph_hits: usize,
    pub graph_files: usize,
    pub graph_context_chars: usize,
    pub vector_hits: usize,
    pub fallback_hits: usize,
    pub fused_vector_hits: usize,
    pub used_vector: bool,
    pub used_fallback: bool,
    pub used_graph: bool,
    pub mock_embedding: bool,
}

pub struct Retriever {
    embedder: Arc<dyn EmbeddingProvider>,
    db: Arc<dyn VectorDB>,
    top_k: usize,
    /// Optional memory store for keyword fallback when vector search returns nothing.
    memory_store: Option<Arc<MemoryStore>>,
    repo_graph_root: Option<PathBuf>,
}

impl Retriever {
    pub fn new(embedder: Arc<dyn EmbeddingProvider>, db: Arc<dyn VectorDB>, top_k: usize) -> Self {
        Self {
            embedder,
            db,
            top_k,
            memory_store: None,
            repo_graph_root: None,
        }
    }

    /// Attach a memory store used as keyword fallback when vector search is empty.
    pub fn with_memory_store(mut self, store: Arc<MemoryStore>) -> Self {
        self.memory_store = Some(store);
        self
    }

    pub fn with_repo_graph_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.repo_graph_root = Some(root.into());
        self
    }

    pub async fn retrieve(&self, query: &str) -> Result<String, MemoryError> {
        let (context, _stats) = self.retrieve_with_stats(query).await?;
        Ok(context)
    }

    pub async fn retrieve_with_stats(
        &self,
        query: &str,
    ) -> Result<(String, RetrieveStats), MemoryError> {
        let mut stats = RetrieveStats {
            query_chars: query.chars().count(),
            ..RetrieveStats::default()
        };
        let (graph_block, graph_files, graph_symbols) = self.graph_context(query, &mut stats);
        let embedding = self
            .embedder
            .embed(query)
            .await
            .map_err(|e| MemoryError::Embedding(e.to_string()))?;

        // Detect mock/zero embeddings — all zeros means the embedder is a stub.
        let is_mock_embedding = embedding.iter().all(|&v| v == 0.0);
        stats.mock_embedding = is_mock_embedding;

        let fragments: Vec<String> = if is_mock_embedding {
            vec![]
        } else {
            let results = self.db.search(embedding, self.top_k).await?;
            stats.vector_hits = results.len();
            let keywords = tokenize_query(query);
            let mut ranked: Vec<(f64, String)> = results
                .iter()
                .filter_map(|r| {
                    let text = r.payload.get("text").and_then(|v| v.as_str())?.to_string();
                    let mut fused_score = r.score as f64;
                    if result_matches_graph_files(&r.id, &graph_files) {
                        fused_score += 0.35;
                    }
                    let text_lower = text.to_lowercase();
                    let symbol_hits = graph_symbols
                        .iter()
                        .filter(|symbol| text_lower.contains(symbol.as_str()))
                        .count();
                    let keyword_hits = keywords
                        .iter()
                        .filter(|kw| text_lower.contains(kw.as_str()))
                        .count();
                    fused_score += symbol_hits as f64 * 0.12;
                    fused_score += keyword_hits as f64 * 0.03;
                    Some((fused_score, text))
                })
                .collect();
            ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
            let mut seen = std::collections::BTreeSet::new();
            let mut out = Vec::new();
            for (_, text) in ranked {
                if seen.insert(text.clone()) {
                    out.push(text);
                }
                if out.len() >= self.top_k {
                    break;
                }
            }
            stats.fused_vector_hits = out.len();
            out
        };

        if !fragments.is_empty() {
            stats.used_vector = true;
            let combined = join_context_blocks([graph_block.as_str(), &fragments.join("\n\n")]);
            return Ok((combined, stats));
        }

        // Fallback: keyword scan across scoped memory files
        if let Some(store) = &self.memory_store {
            let query_lower = query.to_lowercase();
            let keywords: Vec<&str> = query_lower.split_whitespace().collect();
            let mut matched = Vec::new();
            for scope in &["user", "project", "skills"] {
                if let Ok(keys) = store.scoped(scope).list_keys() {
                    for key in &keys {
                        if let Ok(Some(val)) = store.scoped(scope).read(key) {
                            let val_lower = val.to_lowercase();
                            if keywords
                                .iter()
                                .any(|kw| kw.len() >= 3 && val_lower.contains(kw))
                            {
                                stats.fallback_hits += 1;
                                matched.push(format!("[{}/{}]\n{}", scope, key, val));
                            }
                        }
                    }
                }
            }
            if !matched.is_empty() {
                stats.used_fallback = true;
                let combined = join_context_blocks([graph_block.as_str(), &matched.join("\n\n")]);
                return Ok((combined, stats));
            }
        }

        Ok((graph_block, stats))
    }

    fn graph_context(
        &self,
        query: &str,
        stats: &mut RetrieveStats,
    ) -> (String, Vec<String>, Vec<String>) {
        let Some(root) = &self.repo_graph_root else {
            return (String::new(), Vec::new(), Vec::new());
        };
        let Ok(graph) = agent007_core::load_or_build_graph(root, None) else {
            return (String::new(), Vec::new(), Vec::new());
        };
        let bundle = agent007_core::context_bundle_for_query(&graph, query, 6, 2);
        if bundle.text.trim().is_empty() {
            return (String::new(), Vec::new(), Vec::new());
        }
        stats.graph_hits = bundle.matched_symbols.len();
        stats.graph_files = bundle.files.len();
        stats.graph_context_chars = bundle.text.chars().count();
        stats.used_graph = true;
        let graph_files = bundle.files.clone();
        let graph_symbols = bundle
            .matched_symbols
            .iter()
            .map(|node| node.name.to_lowercase())
            .collect();
        (
            format!("[repo_graph]\n{}", bundle.text.trim()),
            graph_files,
            graph_symbols,
        )
    }
}

fn join_context_blocks<'a>(blocks: impl IntoIterator<Item = &'a str>) -> String {
    blocks
        .into_iter()
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn tokenize_query(query: &str) -> Vec<String> {
    query
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
        .map(|token| token.trim().to_lowercase())
        .filter(|token| token.len() >= 3)
        .collect()
}

fn result_matches_graph_files(id: &str, graph_files: &[String]) -> bool {
    let Some(file_path) = id.strip_prefix("file:") else {
        return false;
    };
    graph_files.iter().any(|path| path == file_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MemoryError;
    use crate::vectordb::{SearchResult, VectorDB};
    use agent007_models::{EmbeddingProvider, ModelError};
    use async_trait::async_trait;
    use std::sync::Arc;
    use tempfile::tempdir;

    struct MockEmbeddingProvider;
    #[async_trait]
    impl EmbeddingProvider for MockEmbeddingProvider {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, ModelError> {
            Ok(vec![0.1; 4])
        }
        fn name(&self) -> &str {
            "mock-embed"
        }
    }

    struct MockVectorDB;
    #[async_trait]
    impl VectorDB for MockVectorDB {
        async fn upsert(
            &self,
            _id: &str,
            _vector: Vec<f32>,
            _payload: serde_json::Value,
        ) -> Result<(), MemoryError> {
            Ok(())
        }
        async fn search(
            &self,
            _query: Vec<f32>,
            _limit: usize,
        ) -> Result<Vec<SearchResult>, MemoryError> {
            Ok(vec![
                SearchResult {
                    id: "a".to_string(),
                    score: 0.9,
                    payload: serde_json::json!({ "text": "fragment_alpha" }),
                },
                SearchResult {
                    id: "b".to_string(),
                    score: 0.8,
                    payload: serde_json::json!({ "text": "fragment_beta" }),
                },
            ])
        }
    }

    #[tokio::test]
    async fn retriever_returns_joined_fragments() {
        let embedder = Arc::new(MockEmbeddingProvider);
        let db = Arc::new(MockVectorDB);
        let retriever = Retriever::new(
            embedder as Arc<dyn EmbeddingProvider>,
            db as Arc<dyn VectorDB>,
            2,
        );

        let result = retriever.retrieve("some query").await.unwrap();
        assert!(
            result.contains("fragment_alpha"),
            "result should contain fragment_alpha"
        );
        assert!(
            result.contains("fragment_beta"),
            "result should contain fragment_beta"
        );
    }

    struct GraphAwareVectorDb;
    #[async_trait]
    impl VectorDB for GraphAwareVectorDb {
        async fn upsert(
            &self,
            _id: &str,
            _vector: Vec<f32>,
            _payload: serde_json::Value,
        ) -> Result<(), MemoryError> {
            Ok(())
        }

        async fn search(
            &self,
            _query: Vec<f32>,
            _limit: usize,
        ) -> Result<Vec<SearchResult>, MemoryError> {
            Ok(vec![
                SearchResult {
                    id: "file:src/other.rs".to_string(),
                    score: 0.95,
                    payload: serde_json::json!({ "text": "unrelated text" }),
                },
                SearchResult {
                    id: "file:src/lib.rs".to_string(),
                    score: 0.70,
                    payload: serde_json::json!({ "text": "alpha implementation details" }),
                },
            ])
        }
    }

    #[tokio::test]
    async fn retriever_fuses_graph_context_with_vector_ranking() {
        let root = tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(
            root.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        let embedder = Arc::new(MockEmbeddingProvider);
        let db = Arc::new(GraphAwareVectorDb);
        let retriever = Retriever::new(
            embedder as Arc<dyn EmbeddingProvider>,
            db as Arc<dyn VectorDB>,
            2,
        )
        .with_repo_graph_root(root.path());

        let (context, stats) = retriever.retrieve_with_stats("alpha").await.unwrap();
        assert!(stats.used_graph);
        assert!(stats.graph_hits >= 1);
        assert!(stats.fused_vector_hits >= 1);
        let lib_pos = context
            .find("alpha implementation details")
            .unwrap_or(usize::MAX);
        let other_pos = context.find("unrelated text").unwrap_or(usize::MAX);
        assert!(lib_pos < other_pos, "graph-matching file should rank first");
    }
}
