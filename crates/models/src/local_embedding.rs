use crate::error::ModelError;
use crate::provider::EmbeddingProvider;
use async_trait::async_trait;

/// Lightweight local embedding provider used when no external embedding service
/// is configured. It is lexical rather than semantic, but it keeps RAG useful and
/// fully offline by hashing normalized tokens into a fixed-size vector.
pub struct LocalHashEmbeddingProvider {
    model_name: String,
    dim: usize,
}

impl LocalHashEmbeddingProvider {
    pub fn new(dim: usize) -> Self {
        Self {
            model_name: "local-hash-embed".to_string(),
            dim: dim.max(1),
        }
    }

    pub fn with_name(model_name: impl Into<String>, dim: usize) -> Self {
        Self {
            model_name: model_name.into(),
            dim: dim.max(1),
        }
    }
}

#[async_trait]
impl EmbeddingProvider for LocalHashEmbeddingProvider {
    fn name(&self) -> &str {
        &self.model_name
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, ModelError> {
        let mut vector = vec![0.0f32; self.dim];
        let mut token_count = 0usize;

        for token in tokenize(text) {
            token_count += 1;
            let hash = stable_hash(token.as_bytes());
            let idx = (hash as usize) % self.dim;
            let sign = if (hash >> 63) == 0 { 1.0 } else { -1.0 };
            vector[idx] += sign;
        }

        if token_count > 0 {
            let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut vector {
                    *v /= norm;
                }
            }
        }

        Ok(vector)
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(str::trim)
        .filter(|s| s.len() >= 2)
        .map(|s| s.to_lowercase())
        .collect()
}

fn stable_hash(bytes: &[u8]) -> u64 {
    // FNV-1a 64-bit: deterministic, dependency-free, good enough for feature hashing.
    let mut hash = 0xcbf29ce484222325u64;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_hash_embedding_is_deterministic_and_non_zero() {
        let provider = LocalHashEmbeddingProvider::new(32);
        let a = provider.embed("hello local rag").await.unwrap();
        let b = provider.embed("hello local rag").await.unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.iter().any(|v| *v != 0.0));
    }

    #[tokio::test]
    async fn local_hash_embedding_empty_text_is_zero_vector() {
        let provider = LocalHashEmbeddingProvider::new(16);
        let v = provider.embed("!").await.unwrap();
        assert_eq!(v, vec![0.0; 16]);
    }
}
