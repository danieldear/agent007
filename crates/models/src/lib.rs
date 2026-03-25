pub mod error;
pub mod types;
pub mod provider;
pub mod mock;
pub mod ollama;
pub mod claude;
pub mod codex;
pub mod router;

pub use error::ModelError;
pub use types::{CompletionRequest, CompletionResponse, Message, Role};
pub use provider::{ModelProvider, EmbeddingProvider};
pub use mock::MockProvider;
pub use router::ModelRouter;
pub use claude::ClaudeProvider;
