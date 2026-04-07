# agent007 Plan 1: Workspace + Models + Core

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the Cargo workspace, implement the models crate (Claude/Codex/Ollama/Mock providers + router), and implement the core crate (agent engine, event bus, orchestrator, worker agents) so that agents can be spawned and call real AI models end-to-end.

**Architecture:** Cargo workspace with two library crates (`models`, `core`) and one binary crate (`cli` stub). `models` defines the `ModelProvider` and `EmbeddingProvider` traits with concrete implementations. `core` defines the agent engine: typed events, a `Dispatcher` trait backed by a tokio broadcast stream, a bounded task queue, worker agents, and an orchestrator. All inter-agent communication flows through the `Dispatcher`. External API calls are fully mockable via a hand-written `MockProvider` (no `mockall` — see note in Task 4).

**Tech Stack:** Rust, tokio (full features), tokio-util (CancellationToken/TaskTracker), tokio-stream (BroadcastStream adapter), async-trait, futures, reqwest (rustls-tls), serde/serde_json, thiserror (library errors), anyhow (cli only), uuid, chrono, tracing

**Spec:** `docs/superpowers/specs/2026-03-16-agent007-design.md`

---

## Chunk 1: Workspace Setup + models crate

### File Structure (Chunk 1)

```
agent007/
├── Cargo.toml                          # workspace root
├── crates/
│   ├── cli/
│   │   ├── Cargo.toml
│   │   └── src/main.rs                 # stub binary
│   └── models/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── error.rs                # ModelError (thiserror)
│           ├── types.rs                # CompletionRequest, CompletionResponse, Message, Role
│           ├── provider.rs             # ModelProvider + EmbeddingProvider traits (async-trait)
│           ├── mock.rs                 # MockProvider — hand-written test double
│           ├── ollama.rs               # OllamaProvider (OpenAI-compatible local REST)
│           ├── claude.rs               # ClaudeProvider (Anthropic Messages API)
│           ├── codex.rs                # CodexProvider (OpenAI Chat Completions)
│           └── router.rs               # ModelRouter
```

---

### Task 1: Initialize Cargo workspace

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/cli/Cargo.toml`
- Create: `crates/cli/src/main.rs`
- Create: `crates/models/Cargo.toml`
- Create: `crates/models/src/lib.rs`

- [ ] **Step 1: Create workspace root Cargo.toml**

```toml
# Cargo.toml
[workspace]
members = [
    "crates/cli",
    "crates/models",
]
resolver = "2"

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["rt"] }
tokio-stream = { version = "0.1", features = ["sync"] }
async-trait = "0.1"
futures = "0.3"
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
anyhow = "1"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

Note: `mockall` is intentionally excluded. `ModelProvider` is tested via a hand-written `MockProvider` because `mockall`-generated mocks for `async-trait` traits require additional attribute gymnastics that add noise without benefit at this project scale. If macro-generated mocks are needed later, they can be added per-crate as a dev-dependency.

- [ ] **Step 2: Create cli crate**

```bash
mkdir -p crates/cli/src
```

```toml
# crates/cli/Cargo.toml
[package]
name = "agent007"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

```rust
// crates/cli/src/main.rs
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    println!("agent007 v0.1.0");
    Ok(())
}
```

- [ ] **Step 3: Create models crate**

```bash
mkdir -p crates/models/src
```

```toml
# crates/models/Cargo.toml
[package]
name = "agent007-models"
version = "0.1.0"
edition = "2021"

[dependencies]
async-trait = { workspace = true }
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
```

```rust
// crates/models/src/lib.rs
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
```

- [ ] **Step 4: Verify workspace compiles**

```bash
cd /Users/tvhc84/workspace/rust/agent007
cargo build 2>&1 | head -30
```

Expected: compiles cleanly (error about missing module files is acceptable — they're declared in lib.rs but not yet created)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/
git commit -m "feat: initialize cargo workspace with cli stub and models crate skeleton"
```

---

### Task 2: ModelError and core types

**Files:**
- Create: `crates/models/src/error.rs`
- Create: `crates/models/src/types.rs`

- [ ] **Step 1: Write failing test for types**

Create `crates/models/src/types.rs` with only the test module:

```rust
// crates/models/src/types.rs
#[cfg(test)]
mod tests {
    #[test]
    fn completion_request_serializes() {
        // Will fail until structs are defined below
        assert!(true); // placeholder — real test added with implementation
    }
}
```

- [ ] **Step 2: Run to confirm the module compiles (test is placeholder)**

```bash
cargo test -p agent007-models 2>&1 | head -20
```

Expected: compile error about missing modules (error.rs etc not yet created)

- [ ] **Step 3: Create error.rs**

```rust
// crates/models/src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error from {provider}: {message}")]
    Api { provider: String, message: String },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Provider not configured: {0}")]
    NotConfigured(String),

    #[error("Embedding dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },
}
```

- [ ] **Step 4: Implement types.rs with real tests**

```rust
// crates/models/src/types.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub model: String,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_request_serializes() {
        let req = CompletionRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![Message { role: Role::User, content: "hello".to_string() }],
            max_tokens: Some(100),
            temperature: None,
            system: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("claude-sonnet-4-6"));
        assert!(json.contains("hello"));
        assert!(!json.contains("temperature")); // None fields skipped
    }

    #[test]
    fn completion_response_roundtrips() {
        let resp = CompletionResponse {
            content: "world".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            input_tokens: Some(5),
            output_tokens: Some(1),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: CompletionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, "world");
        assert_eq!(back.input_tokens, Some(5));
    }

    #[test]
    fn role_serializes_lowercase() {
        let json = serde_json::to_string(&Role::User).unwrap();
        assert_eq!(json, "\"user\"");
    }
}
```

- [ ] **Step 5: Create stub files for remaining modules (so lib.rs compiles)**

```bash
touch crates/models/src/provider.rs crates/models/src/mock.rs \
      crates/models/src/ollama.rs crates/models/src/claude.rs \
      crates/models/src/codex.rs crates/models/src/router.rs
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p agent007-models types::tests
```

Expected: 3 tests pass

- [ ] **Step 7: Commit**

```bash
git add crates/models/src/
git commit -m "feat(models): add ModelError, CompletionRequest/Response, Role types"
```

---

### Task 3: ModelProvider and EmbeddingProvider traits

**Files:**
- Modify: `crates/models/src/provider.rs`

- [ ] **Step 1: Write failing test**

```rust
// crates/models/src/provider.rs
use async_trait::async_trait;
use crate::error::ModelError;
use crate::types::{CompletionRequest, CompletionResponse};

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError>;
    fn name(&self) -> &str;
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, ModelError>;
    fn name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    struct AlwaysHelloProvider;

    #[async_trait]
    impl ModelProvider for AlwaysHelloProvider {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
            Ok(CompletionResponse {
                content: "hello".to_string(),
                model: "test".to_string(),
                input_tokens: None,
                output_tokens: None,
            })
        }
        fn name(&self) -> &str { "test" }
    }

    #[tokio::test]
    async fn provider_is_object_safe_and_callable() {
        let provider: Box<dyn ModelProvider> = Box::new(AlwaysHelloProvider);
        let req = CompletionRequest {
            model: "test".to_string(),
            messages: vec![Message { role: Role::User, content: "hi".to_string() }],
            max_tokens: None,
            temperature: None,
            system: None,
        };
        let resp = provider.complete(req).await.unwrap();
        assert_eq!(resp.content, "hello");
        assert_eq!(provider.name(), "test");
    }

    #[tokio::test]
    async fn embedding_provider_is_object_safe() {
        struct ZeroEmbedder;
        #[async_trait]
        impl EmbeddingProvider for ZeroEmbedder {
            async fn embed(&self, _text: &str) -> Result<Vec<f32>, ModelError> {
                Ok(vec![0.0; 4])
            }
            fn name(&self) -> &str { "zero" }
        }
        let ep: Box<dyn EmbeddingProvider> = Box::new(ZeroEmbedder);
        let v = ep.embed("test").await.unwrap();
        assert_eq!(v.len(), 4);
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p agent007-models provider::tests
```

Expected: 2 tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/models/src/provider.rs
git commit -m "feat(models): add ModelProvider and EmbeddingProvider traits (async-trait, object-safe)"
```

---

### Task 4: MockProvider

**Files:**
- Modify: `crates/models/src/mock.rs`

Note: `MockProvider` is a hand-written test double, not `mockall`-generated. This avoids the `#[automock]` + `async-trait` interaction complexity while providing everything needed for tests.

- [ ] **Step 1: Write failing tests first**

```rust
// crates/models/src/mock.rs — test module only, paste at bottom after implementation
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompletionRequest, Message, Role};

    #[tokio::test]
    async fn mock_returns_configured_response() {
        let mock = MockProvider::new("mocked response", "mock-model");
        let req = CompletionRequest {
            model: "any".to_string(),
            messages: vec![Message { role: Role::User, content: "q".to_string() }],
            max_tokens: None,
            temperature: None,
            system: None,
        };
        let resp = mock.complete(req).await.unwrap();
        assert_eq!(resp.content, "mocked response");
        assert_eq!(resp.model, "mock-model");
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn mock_tracks_multiple_calls() {
        let mock = MockProvider::new("resp", "mock");
        let req = CompletionRequest {
            model: "any".to_string(),
            messages: vec![Message { role: Role::User, content: "q".to_string() }],
            max_tokens: None, temperature: None, system: None,
        };
        mock.complete(req.clone()).await.unwrap();
        mock.complete(req.clone()).await.unwrap();
        assert_eq!(mock.call_count(), 2);
    }

    #[tokio::test]
    async fn mock_embedding_returns_zero_vector_of_given_dim() {
        let mock = MockProvider::with_embedding_dim("", "mock", 768);
        let v = mock.embed("hello").await.unwrap();
        assert_eq!(v.len(), 768);
        assert!(v.iter().all(|x| *x == 0.0));
    }
}
```

- [ ] **Step 2: Run to confirm tests fail**

```bash
cargo test -p agent007-models mock::tests 2>&1 | head -15
```

Expected: compile error — MockProvider not defined

- [ ] **Step 3: Implement MockProvider**

```rust
// crates/models/src/mock.rs
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use crate::error::ModelError;
use crate::provider::{EmbeddingProvider, ModelProvider};
use crate::types::{CompletionRequest, CompletionResponse};

pub struct MockProvider {
    response_content: String,
    model_name: String,
    embedding_dim: usize,
    calls: Arc<AtomicUsize>,
}

impl MockProvider {
    pub fn new(response_content: &str, model_name: &str) -> Self {
        Self {
            response_content: response_content.to_string(),
            model_name: model_name.to_string(),
            embedding_dim: 384,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_embedding_dim(response_content: &str, model_name: &str, dim: usize) -> Self {
        let mut p = Self::new(response_content, model_name);
        p.embedding_dim = dim;
        p
    }

    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ModelProvider for MockProvider {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(CompletionResponse {
            content: self.response_content.clone(),
            model: self.model_name.clone(),
            input_tokens: Some(10),
            output_tokens: Some(5),
        })
    }

    fn name(&self) -> &str {
        &self.model_name
    }
}

#[async_trait]
impl EmbeddingProvider for MockProvider {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>, ModelError> {
        Ok(vec![0.0; self.embedding_dim])
    }

    fn name(&self) -> &str {
        &self.model_name
    }
}

// test module from step 1 goes here
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007-models mock::tests
```

Expected: 3 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/models/src/mock.rs
git commit -m "feat(models): add MockProvider test double (hand-written, zero real API calls)"
```

---

### Task 5: OllamaProvider

**Files:**
- Modify: `crates/models/src/ollama.rs`

Note: `name()` returns `"ollama/<model>"`. This requires storing the formatted name at construction time since `&str` must be borrowed from `self`.

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    #[test]
    fn ollama_provider_name_includes_model() {
        let p = OllamaProvider::new("http://localhost:11434", "llama3");
        assert_eq!(p.name(), "ollama/llama3");
    }

    #[test]
    fn ollama_builds_openai_compatible_body() {
        let p = OllamaProvider::new("http://localhost:11434", "llama3");
        let msgs = vec![Message { role: Role::User, content: "hello".to_string() }];
        let body = p.build_body("llama3", &msgs, None, None);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["model"], "llama3");
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hello");
        assert_eq!(v["stream"], false);
    }

    #[test]
    fn ollama_includes_options_when_set() {
        let p = OllamaProvider::new("http://localhost:11434", "llama3");
        let body = p.build_body("llama3", &[], Some(100), Some(0.5));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["options"]["num_predict"], 100);
        assert_eq!(v["options"]["temperature"], 0.5);
    }
}
```

- [ ] **Step 2: Run to confirm fails**

```bash
cargo test -p agent007-models ollama::tests 2>&1 | head -15
```

- [ ] **Step 3: Implement OllamaProvider**

```rust
// crates/models/src/ollama.rs
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::error::ModelError;
use crate::provider::ModelProvider;
use crate::types::{CompletionRequest, CompletionResponse, Message, Role};

pub struct OllamaProvider {
    base_url: String,
    model: String,
    provider_name: String,  // stored as "ollama/<model>" for name() lifetime
    client: reqwest::Client,
}

#[derive(Serialize)]
struct OllamaMsg<'a> { role: &'a str, content: &'a str }

#[derive(Serialize)]
struct OllamaBody<'a> {
    model: &'a str,
    messages: Vec<OllamaMsg<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
}

#[derive(Deserialize)]
struct OllamaResp { message: OllamaRespMsg, model: String,
    #[serde(default)] prompt_eval_count: Option<u32>,
    #[serde(default)] eval_count: Option<u32> }
#[derive(Deserialize)]
struct OllamaRespMsg { content: String }

fn role_str(role: &Role) -> &'static str {
    match role { Role::User => "user", Role::Assistant => "assistant", Role::System => "system" }
}

impl OllamaProvider {
    pub fn new(base_url: &str, model: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            provider_name: format!("ollama/{}", model),
            model: model.to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn build_body(&self, model: &str, messages: &[Message], max_tokens: Option<u32>, temperature: Option<f32>) -> String {
        let msgs: Vec<OllamaMsg> = messages.iter().map(|m| OllamaMsg { role: role_str(&m.role), content: &m.content }).collect();
        serde_json::to_string(&OllamaBody {
            model, messages: msgs, stream: false,
            options: Some(OllamaOptions { temperature, num_predict: max_tokens }),
        }).unwrap()
    }
}

#[async_trait]
impl ModelProvider for OllamaProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let mut messages = request.messages.clone();
        if let Some(sys) = &request.system {
            messages.insert(0, Message { role: Role::System, content: sys.clone() });
        }
        let msgs: Vec<OllamaMsg> = messages.iter().map(|m| OllamaMsg { role: role_str(&m.role), content: &m.content }).collect();
        let body = OllamaBody { model: &self.model, messages: msgs, stream: false,
            options: Some(OllamaOptions { temperature: request.temperature, num_predict: request.max_tokens }) };
        let resp = self.client.post(format!("{}/api/chat", self.base_url)).json(&body).send().await?;
        if !resp.status().is_success() {
            return Err(ModelError::Api { provider: "ollama".into(), message: resp.text().await.unwrap_or_default() });
        }
        let p: OllamaResp = resp.json().await?;
        Ok(CompletionResponse { content: p.message.content, model: p.model,
            input_tokens: p.prompt_eval_count, output_tokens: p.eval_count })
    }

    fn name(&self) -> &str { &self.provider_name }
}

// test module from step 1
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007-models ollama::tests
```

Expected: 3 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/models/src/ollama.rs
git commit -m "feat(models): add OllamaProvider with stored name 'ollama/<model>'"
```

---

### Task 6: ClaudeProvider

**Files:**
- Modify: `crates/models/src/claude.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    #[test]
    fn claude_provider_name() {
        let p = ClaudeProvider::new("key", "claude-sonnet-4-6");
        assert_eq!(p.name(), "claude");
    }

    #[test]
    fn claude_builds_correct_request_body() {
        let p = ClaudeProvider::new("key", "claude-sonnet-4-6");
        let msgs = vec![Message { role: Role::User, content: "hi".to_string() }];
        let body = p.build_body("claude-sonnet-4-6", &msgs, Some(100), None, None);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["model"], "claude-sonnet-4-6");
        assert_eq!(v["max_tokens"], 100);
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hi");
    }

    #[test]
    fn claude_filters_system_messages_from_messages_array() {
        let p = ClaudeProvider::new("key", "claude-sonnet-4-6");
        let msgs = vec![
            Message { role: Role::System, content: "you are helpful".to_string() },
            Message { role: Role::User, content: "hello".to_string() },
        ];
        let body = p.build_body("claude-sonnet-4-6", &msgs, None, None, None);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        // Claude system is a top-level field, not in messages array
        assert_eq!(v["messages"].as_array().unwrap().len(), 1);
        assert_eq!(v["messages"][0]["role"], "user");
    }
}
```

- [ ] **Step 2: Run to confirm fails**

```bash
cargo test -p agent007-models claude::tests 2>&1 | head -15
```

- [ ] **Step 3: Implement ClaudeProvider**

```rust
// crates/models/src/claude.rs
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::error::ModelError;
use crate::provider::ModelProvider;
use crate::types::{CompletionRequest, CompletionResponse, Message, Role};

pub struct ClaudeProvider { api_key: String, model: String, client: reqwest::Client }

#[derive(Serialize)]
struct ClaudeMsg<'a> { role: &'a str, content: &'a str }
#[derive(Serialize)]
struct ClaudeReq<'a> {
    model: &'a str, messages: Vec<ClaudeMsg<'a>>, max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")] system: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")] temperature: Option<f32>,
}
#[derive(Deserialize)]
struct ClaudeResp { content: Vec<ClaudeContent>, model: String, usage: ClaudeUsage }
#[derive(Deserialize)]
struct ClaudeContent { text: String }
#[derive(Deserialize)]
struct ClaudeUsage { input_tokens: u32, output_tokens: u32 }

fn role_str(role: &Role) -> &'static str {
    match role { Role::User => "user", Role::Assistant => "assistant", Role::System => "user" }
}

impl ClaudeProvider {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self { api_key: api_key.to_string(), model: model.to_string(), client: reqwest::Client::new() }
    }

    pub fn build_body(&self, model: &str, messages: &[Message], max_tokens: Option<u32>, temperature: Option<f32>, system: Option<&str>) -> String {
        let msgs: Vec<ClaudeMsg> = messages.iter().filter(|m| m.role != Role::System)
            .map(|m| ClaudeMsg { role: role_str(&m.role), content: &m.content }).collect();
        serde_json::to_string(&ClaudeReq { model, messages: msgs, max_tokens: max_tokens.unwrap_or(4096), system, temperature }).unwrap()
    }
}

#[async_trait]
impl ModelProvider for ClaudeProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let system = request.system.as_deref().or_else(||
            request.messages.iter().find(|m| m.role == Role::System).map(|m| m.content.as_str()));
        let msgs: Vec<ClaudeMsg> = request.messages.iter().filter(|m| m.role != Role::System)
            .map(|m| ClaudeMsg { role: role_str(&m.role), content: &m.content }).collect();
        let body = ClaudeReq { model: &self.model, messages: msgs,
            max_tokens: request.max_tokens.unwrap_or(4096), system, temperature: request.temperature };
        let resp = self.client.post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key).header("anthropic-version", "2023-06-01")
            .json(&body).send().await?;
        if !resp.status().is_success() {
            return Err(ModelError::Api { provider: "claude".into(), message: resp.text().await.unwrap_or_default() });
        }
        let p: ClaudeResp = resp.json().await?;
        Ok(CompletionResponse {
            content: p.content.into_iter().map(|c| c.text).collect::<Vec<_>>().join(""),
            model: p.model, input_tokens: Some(p.usage.input_tokens), output_tokens: Some(p.usage.output_tokens),
        })
    }
    fn name(&self) -> &str { "claude" }
}

// test module from step 1
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007-models claude::tests
```

Expected: 3 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/models/src/claude.rs
git commit -m "feat(models): add ClaudeProvider (Anthropic Messages API)"
```

---

### Task 7: CodexProvider

**Files:**
- Modify: `crates/models/src/codex.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    #[test]
    fn codex_provider_name() {
        let p = CodexProvider::new("key", "gpt-4o");
        assert_eq!(p.name(), "codex");
    }

    #[test]
    fn codex_builds_openai_body() {
        let p = CodexProvider::new("key", "gpt-4o");
        let msgs = vec![Message { role: Role::User, content: "hi".to_string() }];
        let body = p.build_body("gpt-4o", &msgs, Some(50), None);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["model"], "gpt-4o");
        assert_eq!(v["max_tokens"], 50);
        assert_eq!(v["messages"][0]["role"], "user");
    }
}
```

- [ ] **Step 2: Run to confirm fails**

```bash
cargo test -p agent007-models codex::tests 2>&1 | head -10
```

- [ ] **Step 3: Implement CodexProvider**

```rust
// crates/models/src/codex.rs
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::error::ModelError;
use crate::provider::ModelProvider;
use crate::types::{CompletionRequest, CompletionResponse, Message, Role};

pub struct CodexProvider { api_key: String, model: String, client: reqwest::Client }

#[derive(Serialize)]
struct OAIMsg<'a> { role: &'a str, content: &'a str }
#[derive(Serialize)]
struct OAIReq<'a> {
    model: &'a str, messages: Vec<OAIMsg<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")] max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")] temperature: Option<f32>,
}
#[derive(Deserialize)]
struct OAIResp { choices: Vec<OAIChoice>, model: String, usage: Option<OAIUsage> }
#[derive(Deserialize)]
struct OAIChoice { message: OAIRespMsg }
#[derive(Deserialize)]
struct OAIRespMsg { content: String }
#[derive(Deserialize)]
struct OAIUsage { prompt_tokens: u32, completion_tokens: u32 }

fn role_str(role: &Role) -> &'static str {
    match role { Role::User => "user", Role::Assistant => "assistant", Role::System => "system" }
}

impl CodexProvider {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self { api_key: api_key.to_string(), model: model.to_string(), client: reqwest::Client::new() }
    }

    pub fn build_body(&self, model: &str, messages: &[Message], max_tokens: Option<u32>, temperature: Option<f32>) -> String {
        let msgs: Vec<OAIMsg> = messages.iter().map(|m| OAIMsg { role: role_str(&m.role), content: &m.content }).collect();
        serde_json::to_string(&OAIReq { model, messages: msgs, max_tokens, temperature }).unwrap()
    }
}

#[async_trait]
impl ModelProvider for CodexProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let mut messages = request.messages.clone();
        if let Some(sys) = &request.system {
            messages.insert(0, Message { role: Role::System, content: sys.clone() });
        }
        let msgs: Vec<OAIMsg> = messages.iter().map(|m| OAIMsg { role: role_str(&m.role), content: &m.content }).collect();
        let body = OAIReq { model: &self.model, messages: msgs, max_tokens: request.max_tokens, temperature: request.temperature };
        let resp = self.client.post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key).json(&body).send().await?;
        if !resp.status().is_success() {
            return Err(ModelError::Api { provider: "codex".into(), message: resp.text().await.unwrap_or_default() });
        }
        let p: OAIResp = resp.json().await?;
        let content = p.choices.into_iter().next().map(|c| c.message.content).unwrap_or_default();
        Ok(CompletionResponse { content, model: p.model,
            input_tokens: p.usage.as_ref().map(|u| u.prompt_tokens),
            output_tokens: p.usage.as_ref().map(|u| u.completion_tokens) })
    }
    fn name(&self) -> &str { "codex" }
}

// test module from step 1
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007-models codex::tests
```

Expected: 2 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/models/src/codex.rs
git commit -m "feat(models): add CodexProvider (OpenAI Chat Completions)"
```

---

### Task 8: ModelRouter

**Files:**
- Modify: `crates/models/src/router.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockProvider;
    use std::sync::Arc;

    fn make_router() -> ModelRouter {
        let mut r = ModelRouter::new("claude");
        r.register("claude", Arc::new(MockProvider::new("claude-resp", "claude")));
        r.register("codex", Arc::new(MockProvider::new("codex-resp", "codex")));
        r.register("ollama", Arc::new(MockProvider::new("ollama-resp", "ollama")));
        r
    }

    #[test]
    fn router_falls_back_to_default() {
        let r = make_router();
        assert_eq!(r.route("unknown").name(), "claude");
    }

    #[test]
    fn router_picks_rule_over_default() {
        let mut r = make_router();
        r.add_rule("code_completion", "codex");
        assert_eq!(r.route("code_completion").name(), "codex");
        assert_eq!(r.route("reasoning").name(), "claude");  // no rule → default
    }

    #[tokio::test]
    async fn router_routes_to_correct_provider_output() {
        use crate::types::{CompletionRequest, Message, Role};
        let mut r = make_router();
        r.add_rule("code_completion", "codex");
        let resp = r.route("code_completion").complete(CompletionRequest {
            model: "any".into(),
            messages: vec![Message { role: Role::User, content: "write code".into() }],
            max_tokens: None, temperature: None, system: None,
        }).await.unwrap();
        assert_eq!(resp.content, "codex-resp");
    }
}
```

- [ ] **Step 2: Run to confirm fails**

```bash
cargo test -p agent007-models router::tests 2>&1 | head -15
```

- [ ] **Step 3: Implement ModelRouter**

```rust
// crates/models/src/router.rs
use std::collections::HashMap;
use std::sync::Arc;
use crate::provider::ModelProvider;

pub struct ModelRouter {
    providers: HashMap<String, Arc<dyn ModelProvider>>,
    rules: HashMap<String, String>,
    default: String,
}

impl ModelRouter {
    pub fn new(default_provider: &str) -> Self {
        Self { providers: HashMap::new(), rules: HashMap::new(), default: default_provider.to_string() }
    }

    pub fn register(&mut self, name: &str, provider: Arc<dyn ModelProvider>) {
        self.providers.insert(name.to_string(), provider);
    }

    pub fn add_rule(&mut self, task_type: &str, provider_name: &str) {
        self.rules.insert(task_type.to_string(), provider_name.to_string());
    }

    pub fn route(&self, task_type: &str) -> Arc<dyn ModelProvider> {
        let name = self.rules.get(task_type).unwrap_or(&self.default);
        self.providers.get(name)
            .or_else(|| self.providers.get(&self.default))
            .expect("default provider must be registered before routing")
            .clone()
    }
}

// test module from step 1
```

- [ ] **Step 4: Run all models tests**

```bash
cargo test -p agent007-models
```

Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/models/src/router.rs
git commit -m "feat(models): add ModelRouter with task-type routing rules"
```

---

## Chunk 2: core crate

### File Structure (Chunk 2)

```
crates/core/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── error.rs          # CoreError (thiserror)
    ├── types.rs          # AgentId, PromptRef, MemoryRef, PromptStore (Arc<Mutex>)
    ├── task.rs           # Task, TaskResult, TaskQueue  ← created BEFORE events.rs
    ├── events.rs         # AgentEvent enum              ← depends on task.rs
    ├── dispatcher.rs     # Dispatcher trait + LocalDispatcher (Stream-based)
    ├── agent.rs          # AgentHandle, AgentState
    ├── worker.rs         # WorkerAgent — calls model, emits events, shared PromptStore
    └── orchestrator.rs   # OrchestratorAgent — routes and runs tasks
```

Note: `task.rs` is created before `events.rs` because `AgentEvent` references `Task` and `TaskResult`.

---

### Task 9: core crate scaffold

**Files:**
- Create: `crates/core/Cargo.toml`
- Create: `crates/core/src/lib.rs`
- Modify: `Cargo.toml` (add core to workspace members)

- [ ] **Step 1: Add core to workspace Cargo.toml members**

Edit root `Cargo.toml`, add `"crates/core"` to the `members` array.

- [ ] **Step 2: Create core Cargo.toml**

```toml
# crates/core/Cargo.toml
[package]
name = "agent007-core"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-models = { path = "../models" }
async-trait = { workspace = true }
tokio = { workspace = true }
tokio-util = { workspace = true }
tokio-stream = { workspace = true }
futures = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
```

- [ ] **Step 3: Create stub source files**

```bash
mkdir -p crates/core/src
touch crates/core/src/error.rs crates/core/src/types.rs crates/core/src/task.rs \
      crates/core/src/events.rs crates/core/src/dispatcher.rs crates/core/src/agent.rs \
      crates/core/src/worker.rs crates/core/src/orchestrator.rs
```

- [ ] **Step 4: Create lib.rs**

```rust
// crates/core/src/lib.rs
pub mod error;
pub mod types;
pub mod task;
pub mod events;
pub mod dispatcher;
pub mod agent;
pub mod worker;
pub mod orchestrator;

pub use error::CoreError;
pub use types::{AgentId, PromptRef, MemoryRef, PromptStore};
pub use task::{Task, TaskResult, TaskQueue};
pub use events::AgentEvent;
pub use dispatcher::{Dispatcher, LocalDispatcher};
```

- [ ] **Step 5: Verify workspace builds (stub files, no logic yet)**

```bash
cargo build --workspace 2>&1 | head -20
```

Expected: may warn about empty files — that's fine at this point

- [ ] **Step 6: Commit**

```bash
git add crates/core/ Cargo.toml
git commit -m "feat(core): scaffold crate structure and Cargo.toml"
```

---

### Task 10: CoreError and types

**Files:**
- Modify: `crates/core/src/error.rs`
- Modify: `crates/core/src/types.rs`

- [ ] **Step 1: Write failing test**

```rust
// Add to crates/core/src/error.rs
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn core_error_displays_agent_id() {
        let e = CoreError::AgentNotFound("abc-123".to_string());
        assert!(e.to_string().contains("abc-123"));
    }
}
```

- [ ] **Step 2: Run to confirm fails**

```bash
cargo test -p agent007-core error::tests 2>&1 | head -10
```

- [ ] **Step 3: Implement error.rs**

```rust
// crates/core/src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Task queue full — backpressure limit reached")]
    TaskQueueFull,

    #[error("Dispatcher publish failed: {0}")]
    DispatchFailed(String),

    #[error("Model error: {0}")]
    Model(#[from] agent007_models::ModelError),

    #[error("Shutdown in progress")]
    ShuttingDown,
}
```

- [ ] **Step 4: Implement types.rs**

```rust
// crates/core/src/types.rs
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentId(pub Uuid);

impl AgentId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

impl Default for AgentId {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Opaque ref to a prompt stored in PromptStore. Never put raw prompts on the event bus.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PromptRef(pub Uuid);

impl PromptRef {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// Opaque ref to a memory value. Never put raw memory content on the event bus.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MemoryRef(pub Uuid);

impl MemoryRef {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// Shared, thread-safe store mapping PromptRef → raw prompt text.
/// Must be Arc<Mutex<PromptStore>> so that PromptRef values remain resolvable
/// after the call that created them.
#[derive(Default)]
pub struct PromptStore {
    inner: HashMap<PromptRef, String>,
}

impl PromptStore {
    pub fn insert(&mut self, prompt: String) -> PromptRef {
        let r = PromptRef::new();
        self.inner.insert(r.clone(), prompt);
        r
    }

    pub fn get(&self, r: &PromptRef) -> Option<&str> {
        self.inner.get(r).map(|s| s.as_str())
    }
}

/// Convenience alias used by WorkerAgent.
pub type SharedPromptStore = Arc<Mutex<PromptStore>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_store_insert_and_retrieve() {
        let mut store = PromptStore::default();
        let r = store.insert("my prompt".to_string());
        assert_eq!(store.get(&r), Some("my prompt"));
    }

    #[test]
    fn shared_prompt_store_accessible_across_clone() {
        let store: SharedPromptStore = Arc::new(Mutex::new(PromptStore::default()));
        let r = store.lock().unwrap().insert("shared prompt".to_string());
        let store2 = Arc::clone(&store);
        assert_eq!(store2.lock().unwrap().get(&r), Some("shared prompt"));
    }
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p agent007-core error::tests types::tests
```

Expected: 3 tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/error.rs crates/core/src/types.rs
git commit -m "feat(core): add CoreError, AgentId, PromptRef, MemoryRef, shared PromptStore"
```

---

### Task 11: Task and TaskQueue

**Files:**
- Modify: `crates/core/src/task.rs`

Note: task.rs is implemented before events.rs because AgentEvent references Task and TaskResult.

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_queue_send_and_receive() {
        let (queue, mut rx) = TaskQueue::new(8);
        let task = Task::new("write a function");
        queue.send(task).await.unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.description, "write a function");
        assert_eq!(received.task_type, "default");
    }

    #[tokio::test]
    async fn task_queue_respects_capacity_limit() {
        let (queue, _rx) = TaskQueue::new(2);
        queue.send(Task::new("t1")).await.unwrap();
        queue.send(Task::new("t2")).await.unwrap();
        // try_send on a full channel with no receiver should fail
        assert!(queue.try_send(Task::new("t3")).is_err());
    }

    #[test]
    fn task_result_success_sets_flag() {
        let id = uuid::Uuid::new_v4();
        let r = TaskResult::success(id, "output".to_string());
        assert!(r.success);
        assert_eq!(r.task_id, id);
    }

    #[test]
    fn task_result_failure_clears_flag() {
        let id = uuid::Uuid::new_v4();
        let r = TaskResult::failure(id, "oops".to_string());
        assert!(!r.success);
        assert_eq!(r.output, "oops");
    }
}
```

- [ ] **Step 2: Run to confirm fails**

```bash
cargo test -p agent007-core task::tests 2>&1 | head -10
```

- [ ] **Step 3: Implement task.rs**

```rust
// crates/core/src/task.rs
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::error::CoreError;
use crate::types::AgentId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub description: String,
    pub assigned_to: Option<AgentId>,
    pub task_type: String,
}

impl Task {
    pub fn new(description: &str) -> Self {
        Self { id: Uuid::new_v4(), description: description.to_string(), assigned_to: None, task_type: "default".to_string() }
    }

    pub fn with_type(mut self, task_type: &str) -> Self {
        self.task_type = task_type.to_string();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: Uuid,
    pub output: String,
    pub success: bool,
}

impl TaskResult {
    pub fn success(task_id: Uuid, output: String) -> Self { Self { task_id, output, success: true } }
    pub fn failure(task_id: Uuid, reason: String) -> Self { Self { task_id, output: reason, success: false } }
}

pub struct TaskQueue {
    sender: tokio::sync::mpsc::Sender<Task>,
}

impl TaskQueue {
    pub fn new(capacity: usize) -> (Self, tokio::sync::mpsc::Receiver<Task>) {
        let (tx, rx) = tokio::sync::mpsc::channel(capacity);
        (Self { sender: tx }, rx)
    }

    pub async fn send(&self, task: Task) -> Result<(), CoreError> {
        self.sender.send(task).await.map_err(|_| CoreError::TaskQueueFull)
    }

    pub fn try_send(&self, task: Task) -> Result<(), CoreError> {
        self.sender.try_send(task).map_err(|_| CoreError::TaskQueueFull)
    }
}

// test module from step 1
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007-core task::tests
```

Expected: 4 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/task.rs
git commit -m "feat(core): add Task, TaskResult, bounded TaskQueue"
```

---

### Task 12: AgentEvent and Dispatcher

**Files:**
- Modify: `crates/core/src/events.rs`
- Modify: `crates/core/src/dispatcher.rs`

- [ ] **Step 1: Write failing test for events**

```rust
// events.rs test module
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentId, PromptRef};

    #[test]
    fn agent_event_clones_cleanly() {
        let e = AgentEvent::ModelRequest {
            provider: "claude".to_string(),
            prompt_ref: PromptRef::new(),
            token_estimate: 100,
        };
        let c = e.clone();
        assert!(matches!(c, AgentEvent::ModelRequest { token_estimate: 100, .. }));
    }

    #[test]
    fn memory_write_uses_opaque_ref() {
        let e = AgentEvent::MemoryWrite { key: "user.md".to_string(), value_ref: crate::types::MemoryRef::new() };
        // Confirm no raw value is on the event — just a ref
        if let AgentEvent::MemoryWrite { key, .. } = e {
            assert_eq!(key, "user.md");
        }
    }
}
```

- [ ] **Step 2: Run to confirm fails**

```bash
cargo test -p agent007-core events::tests 2>&1 | head -10
```

- [ ] **Step 3: Implement events.rs**

```rust
// crates/core/src/events.rs
// AgentEvent carries NO raw prompt text or memory values — only opaque refs.
// Inner types (ToolCall, HookEvent) do NOT derive Serialize/Deserialize in Phase 1.
// AgentEvent must be Clone + Send + 'static for broadcast::Sender<AgentEvent>.
use crate::task::{Task, TaskResult};
use crate::types::{AgentId, MemoryRef, PromptRef};

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum HookEvent {
    PreAgentRun,
    PostAgentRun,
    PreToolCall,
    PostToolCall,
    OnMemoryWrite,
    OnSkillExecute,
    PostTaskComplete,
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    TaskAssigned { agent_id: AgentId, task: Task },
    TaskCompleted { agent_id: AgentId, result: TaskResult },
    ToolCall { agent_id: AgentId, tool: ToolCall },
    MemoryWrite { key: String, value_ref: MemoryRef },  // value_ref is opaque — no raw value
    HookFired { event: HookEvent },
    ModelRequest { provider: String, prompt_ref: PromptRef, token_estimate: usize },
}

// test module from step 1
```

- [ ] **Step 4: Write failing test for Dispatcher**

```rust
// dispatcher.rs test module
#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::AgentEvent;
    use crate::types::PromptRef;
    use futures::StreamExt;

    #[tokio::test]
    async fn local_dispatcher_publish_then_receive() {
        let d = LocalDispatcher::new(64);
        let mut stream = d.subscribe().await.unwrap();

        d.publish(AgentEvent::ModelRequest {
            provider: "claude".to_string(),
            prompt_ref: PromptRef::new(),
            token_estimate: 42,
        }).await.unwrap();

        let received = stream.next().await.unwrap();
        assert!(matches!(received, AgentEvent::ModelRequest { token_estimate: 42, .. }));
    }

    #[tokio::test]
    async fn dispatcher_delivers_to_multiple_subscribers() {
        let d = LocalDispatcher::new(64);
        let mut s1 = d.subscribe().await.unwrap();
        let mut s2 = d.subscribe().await.unwrap();

        d.publish(AgentEvent::ModelRequest {
            provider: "ollama".into(),
            prompt_ref: PromptRef::new(),
            token_estimate: 7,
        }).await.unwrap();

        let e1 = s1.next().await.unwrap();
        let e2 = s2.next().await.unwrap();
        assert!(matches!(e1, AgentEvent::ModelRequest { token_estimate: 7, .. }));
        assert!(matches!(e2, AgentEvent::ModelRequest { token_estimate: 7, .. }));
    }
}
```

- [ ] **Step 5: Run to confirm fails**

```bash
cargo test -p agent007-core dispatcher::tests 2>&1 | head -10
```

- [ ] **Step 6: Implement dispatcher.rs**

```rust
// crates/core/src/dispatcher.rs
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;
use crate::error::CoreError;
use crate::events::AgentEvent;

pub type EventStream = Pin<Box<dyn Stream<Item = AgentEvent> + Send>>;

#[async_trait]
pub trait Dispatcher: Send + Sync {
    async fn publish(&self, event: AgentEvent) -> Result<(), CoreError>;
    /// Returns a stream of events. Uses Pin<Box<dyn Stream>> not broadcast::Receiver
    /// so this trait is swappable for Phase 3 network transports.
    async fn subscribe(&self) -> Result<EventStream, CoreError>;
}

pub struct LocalDispatcher {
    sender: Arc<broadcast::Sender<AgentEvent>>,
}

impl LocalDispatcher {
    pub fn new(capacity: usize) -> Arc<Self> {
        let (tx, _) = broadcast::channel(capacity);
        Arc::new(Self { sender: Arc::new(tx) })
    }
}

#[async_trait]
impl Dispatcher for LocalDispatcher {
    async fn publish(&self, event: AgentEvent) -> Result<(), CoreError> {
        let _ = self.sender.send(event); // Ignore "no receivers" error at startup
        Ok(())
    }

    async fn subscribe(&self) -> Result<EventStream, CoreError> {
        let rx = self.sender.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|r| async move { r.ok() });
        Ok(Box::pin(stream))
    }
}

// test module from step 1
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p agent007-core events::tests dispatcher::tests
```

Expected: 4 tests pass

- [ ] **Step 8: Commit**

```bash
git add crates/core/src/events.rs crates/core/src/dispatcher.rs
git commit -m "feat(core): add AgentEvent (opaque refs, no raw data) and Dispatcher/LocalDispatcher"
```

---

### Task 13: WorkerAgent

**Files:**
- Modify: `crates/core/src/agent.rs`
- Modify: `crates/core/src/worker.rs`

- [ ] **Step 1: Write failing test**

```rust
// worker.rs test module
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::LocalDispatcher;
    use crate::events::AgentEvent;
    use crate::task::Task;
    use agent007_models::MockProvider;
    use futures::StreamExt;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn worker_executes_task_via_provider() {
        let d = LocalDispatcher::new(32);
        let mut events = d.subscribe().await.unwrap();
        let provider = Arc::new(MockProvider::new("task done", "mock"));
        let store = Arc::new(Mutex::new(crate::types::PromptStore::default()));
        let token = CancellationToken::new();

        let worker = WorkerAgent::new(
            Arc::clone(&d) as Arc<dyn crate::dispatcher::Dispatcher>,
            Arc::clone(&provider) as Arc<dyn agent007_models::ModelProvider>,
            store,
            token,
        );

        let task = Task::new("do something");
        let task_id = task.id;
        let result = worker.execute(task).await.unwrap();

        assert!(result.success);
        assert_eq!(result.task_id, task_id);
        assert_eq!(result.output, "task done");
        assert_eq!(provider.call_count(), 1);

        // Verify ModelRequest event emitted with opaque ref (no raw prompt)
        let event = events.next().await.unwrap();
        assert!(matches!(event, AgentEvent::ModelRequest { .. }));
    }

    #[tokio::test]
    async fn worker_prompt_ref_is_resolvable_in_store() {
        let d = LocalDispatcher::new(32);
        let mut events = d.subscribe().await.unwrap();
        let provider = Arc::new(MockProvider::new("result", "mock"));
        let store = Arc::new(Mutex::new(crate::types::PromptStore::default()));
        let store_ref = Arc::clone(&store);
        let token = CancellationToken::new();

        let worker = WorkerAgent::new(
            Arc::clone(&d) as Arc<dyn crate::dispatcher::Dispatcher>,
            provider,
            store,
            token,
        );

        worker.execute(Task::new("my prompt text")).await.unwrap();

        // The PromptRef on the event must resolve to the actual prompt
        let event = events.next().await.unwrap();
        if let AgentEvent::ModelRequest { prompt_ref, .. } = event {
            let locked = store_ref.lock().unwrap();
            assert_eq!(locked.get(&prompt_ref), Some("my prompt text"));
        } else {
            panic!("expected ModelRequest event");
        }
    }

    #[tokio::test]
    async fn worker_returns_shutdown_error_when_cancelled() {
        let d = LocalDispatcher::new(32);
        let provider = Arc::new(MockProvider::new("", "mock"));
        let store = Arc::new(Mutex::new(crate::types::PromptStore::default()));
        let token = CancellationToken::new();
        token.cancel();
        let worker = WorkerAgent::new(
            Arc::clone(&d) as Arc<dyn crate::dispatcher::Dispatcher>,
            provider,
            store,
            token,
        );
        let result = worker.execute(Task::new("any")).await;
        assert!(matches!(result, Err(crate::error::CoreError::ShuttingDown)));
    }
}
```

- [ ] **Step 2: Run to confirm fails**

```bash
cargo test -p agent007-core worker::tests 2>&1 | head -15
```

- [ ] **Step 3: Implement agent.rs**

```rust
// crates/core/src/agent.rs
use crate::types::AgentId;

#[derive(Debug, Clone, PartialEq)]
pub enum AgentState { Idle, Running, Done, Failed(String) }

pub struct AgentHandle {
    pub id: AgentId,
    pub state: AgentState,
}

impl AgentHandle {
    pub fn new() -> Self { Self { id: AgentId::new(), state: AgentState::Idle } }
}
```

- [ ] **Step 4: Implement worker.rs**

```rust
// crates/core/src/worker.rs
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use agent007_models::{CompletionRequest, Message, ModelProvider, Role};
use crate::dispatcher::Dispatcher;
use crate::error::CoreError;
use crate::events::AgentEvent;
use crate::task::{Task, TaskResult};
use crate::types::{AgentId, SharedPromptStore};

pub struct WorkerAgent {
    pub id: AgentId,
    dispatcher: Arc<dyn Dispatcher>,
    provider: Arc<dyn ModelProvider>,
    prompt_store: SharedPromptStore,
    cancellation: CancellationToken,
}

impl WorkerAgent {
    pub fn new(
        dispatcher: Arc<dyn Dispatcher>,
        provider: Arc<dyn ModelProvider>,
        prompt_store: SharedPromptStore,
        cancellation: CancellationToken,
    ) -> Self {
        Self { id: AgentId::new(), dispatcher, provider, prompt_store, cancellation }
    }

    pub async fn execute(&self, task: Task) -> Result<TaskResult, CoreError> {
        if self.cancellation.is_cancelled() {
            return Err(CoreError::ShuttingDown);
        }

        // Store prompt in shared PromptStore, emit opaque ref on event bus
        let prompt_ref = self.prompt_store.lock().unwrap().insert(task.description.clone());

        self.dispatcher.publish(AgentEvent::ModelRequest {
            provider: self.provider.name().to_string(),
            prompt_ref: prompt_ref.clone(),
            token_estimate: task.description.split_whitespace().count().saturating_mul(2),
        }).await?;

        let request = CompletionRequest {
            model: self.provider.name().to_string(),
            messages: vec![Message { role: Role::User, content: task.description.clone() }],
            max_tokens: Some(4096),
            temperature: Some(0.7),
            system: None,
        };

        let response = self.provider.complete(request).await?;

        let result = TaskResult::success(task.id, response.content);
        self.dispatcher.publish(AgentEvent::TaskCompleted {
            agent_id: self.id.clone(),
            result: result.clone(),
        }).await?;

        Ok(result)
    }
}

// test module from step 1
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p agent007-core worker::tests
```

Expected: 3 tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/agent.rs crates/core/src/worker.rs
git commit -m "feat(core): add WorkerAgent with shared PromptStore, opaque refs on event bus"
```

---

### Task 14: OrchestratorAgent

**Files:**
- Modify: `crates/core/src/orchestrator.rs`

- [ ] **Step 1: Write failing tests**

```rust
// orchestrator.rs test module
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::LocalDispatcher;
    use crate::events::AgentEvent;
    use crate::task::Task;
    use agent007_models::{MockProvider, ModelRouter};
    use futures::StreamExt;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn make_orchestrator(response: &str, token: CancellationToken) -> (OrchestratorAgent, Arc<LocalDispatcher>) {
        let d = LocalDispatcher::new(64);
        let mock = Arc::new(MockProvider::new(response, "mock"));
        let mut router = ModelRouter::new("mock");
        router.register("mock", Arc::clone(&mock) as Arc<dyn agent007_models::ModelProvider>);
        let store = Arc::new(Mutex::new(crate::types::PromptStore::default()));
        let orch = OrchestratorAgent::new(
            Arc::clone(&d) as Arc<dyn crate::dispatcher::Dispatcher>,
            Arc::new(router), store, token, 4,
        );
        (orch, d)
    }

    #[tokio::test]
    async fn orchestrator_returns_successful_result() {
        let token = CancellationToken::new();
        let (orch, _d) = make_orchestrator("the answer", token);
        let result = orch.run(Task::new("what is 6*7?")).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output, "the answer");
    }

    #[tokio::test]
    async fn orchestrator_emits_model_request_and_task_completed_events() {
        let token = CancellationToken::new();
        let (orch, d) = make_orchestrator("done", token);
        let mut events = d.subscribe().await.unwrap();

        orch.run(Task::new("build something")).await.unwrap();

        let e1 = events.next().await.unwrap();
        let e2 = events.next().await.unwrap();
        assert!(matches!(e1, AgentEvent::ModelRequest { .. }));
        assert!(matches!(e2, AgentEvent::TaskCompleted { .. }));
    }

    #[tokio::test]
    async fn orchestrator_respects_cancellation() {
        let token = CancellationToken::new();
        token.cancel();
        let (orch, _d) = make_orchestrator("", token);
        let result = orch.run(Task::new("any")).await;
        assert!(matches!(result, Err(crate::error::CoreError::ShuttingDown)));
    }
}
```

- [ ] **Step 2: Run to confirm fails**

```bash
cargo test -p agent007-core orchestrator::tests 2>&1 | head -15
```

- [ ] **Step 3: Implement orchestrator.rs**

```rust
// crates/core/src/orchestrator.rs
// Phase 1: single-worker orchestrator — routes one task to one worker.
// Phase 2 will decompose tasks into subtasks across multiple parallel workers.
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use agent007_models::ModelRouter;
use crate::dispatcher::Dispatcher;
use crate::error::CoreError;
use crate::task::{Task, TaskResult};
use crate::types::{PromptStore, SharedPromptStore};
use crate::worker::WorkerAgent;

pub struct OrchestratorAgent {
    dispatcher: Arc<dyn Dispatcher>,
    router: Arc<ModelRouter>,
    prompt_store: SharedPromptStore,
    cancellation: CancellationToken,
    _max_workers: usize,
}

impl OrchestratorAgent {
    pub fn new(
        dispatcher: Arc<dyn Dispatcher>,
        router: Arc<ModelRouter>,
        prompt_store: SharedPromptStore,
        cancellation: CancellationToken,
        max_workers: usize,
    ) -> Self {
        Self { dispatcher, router, prompt_store, cancellation, _max_workers: max_workers }
    }

    pub async fn run(&self, task: Task) -> Result<TaskResult, CoreError> {
        if self.cancellation.is_cancelled() {
            return Err(CoreError::ShuttingDown);
        }
        let provider = self.router.route(&task.task_type);
        let worker = WorkerAgent::new(
            Arc::clone(&self.dispatcher),
            provider,
            Arc::clone(&self.prompt_store),
            self.cancellation.clone(),
        );
        worker.execute(task).await
    }
}

// test module from step 1
```

- [ ] **Step 4: Run all core tests**

```bash
cargo test -p agent007-core
```

Expected: all pass

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/orchestrator.rs
git commit -m "feat(core): add OrchestratorAgent — routes tasks, verifies event bus end-to-end"
```

---

### Task 15: Wire CLI — end-to-end smoke test

**Files:**
- Modify: `crates/cli/Cargo.toml`
- Modify: `crates/cli/src/main.rs`
- Create: `crates/cli/tests/smoke_test.rs`

- [ ] **Step 1: Update cli Cargo.toml**

```toml
# crates/cli/Cargo.toml
[package]
name = "agent007"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-core = { path = "../core" }
agent007-models = { path = "../models" }
anyhow = { workspace = true }
tokio = { workspace = true }
tokio-util = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }

[dev-dependencies]
futures = { workspace = true }
```

- [ ] **Step 2: Write failing smoke test that verifies event bus**

```rust
// crates/cli/tests/smoke_test.rs
use agent007_core::{
    dispatcher::{Dispatcher, LocalDispatcher},
    events::AgentEvent,
    orchestrator::OrchestratorAgent,
    task::Task,
    types::PromptStore,
};
use agent007_models::{MockProvider, ModelProvider, ModelRouter};
use futures::StreamExt;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn smoke_test_agents_run_and_emit_events() {
    let d = LocalDispatcher::new(64);
    let mut events = d.subscribe().await.unwrap();

    let mock = Arc::new(MockProvider::new("42", "mock"));
    let mut router = ModelRouter::new("mock");
    router.register("mock", Arc::clone(&mock) as Arc<dyn ModelProvider>);
    let store = Arc::new(Mutex::new(PromptStore::default()));
    let token = CancellationToken::new();

    let orch = OrchestratorAgent::new(
        Arc::clone(&d) as Arc<dyn Dispatcher>,
        Arc::new(router),
        store,
        token,
        4,
    );

    let result = orch.run(Task::new("what is 6 times 7?")).await.unwrap();

    // Verify result
    assert!(result.success);
    assert_eq!(result.output, "42");

    // Verify ModelRequest event emitted with opaque prompt ref (no raw prompt text)
    let e1 = events.next().await.unwrap();
    assert!(matches!(e1, AgentEvent::ModelRequest { .. }),
        "expected ModelRequest event, got {:?}", e1);

    // Verify TaskCompleted event emitted
    let e2 = events.next().await.unwrap();
    assert!(matches!(e2, AgentEvent::TaskCompleted { .. }),
        "expected TaskCompleted event, got {:?}", e2);

    // Verify the model was actually called (not short-circuited)
    assert_eq!(mock.call_count(), 1);
}
```

- [ ] **Step 3: Run to confirm fails**

```bash
cargo test -p agent007 smoke_test 2>&1 | head -15
```

- [ ] **Step 4: Update main.rs**

```rust
// crates/cli/src/main.rs
use agent007_core::{
    dispatcher::{Dispatcher, LocalDispatcher},
    orchestrator::OrchestratorAgent,
    task::Task,
    types::PromptStore,
};
use agent007_models::{MockProvider, ModelProvider, ModelRouter};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let d = LocalDispatcher::new(256);
    let mock = Arc::new(MockProvider::new("Hello from agent007!", "mock"));
    let mut router = ModelRouter::new("mock");
    router.register("mock", Arc::clone(&mock) as Arc<dyn ModelProvider>);
    let store = Arc::new(Mutex::new(PromptStore::default()));
    let token = CancellationToken::new();

    let orch = OrchestratorAgent::new(
        Arc::clone(&d) as Arc<dyn Dispatcher>,
        Arc::new(router),
        store,
        token,
        4,
    );

    let result = orch.run(Task::new("say hello")).await?;
    println!("Result: {}", result.output);
    Ok(())
}
```

- [ ] **Step 5: Run smoke test**

```bash
cargo test -p agent007 smoke_test
```

Expected: passes

- [ ] **Step 6: Run the binary**

```bash
cargo run -p agent007
```

Expected: `Result: Hello from agent007!`

- [ ] **Step 7: Run full workspace tests**

```bash
cargo test --workspace
```

Expected: all tests pass

- [ ] **Step 8: Final commit**

```bash
git add crates/cli/
git commit -m "feat(cli): smoke test verifies full event bus path end-to-end"
```

---

## Summary

Plan 1 delivers:
- Cargo workspace with `models`, `core`, `cli` crates and all dependencies pinned via `[workspace.dependencies]`
- `ModelProvider` + `EmbeddingProvider` traits (async-trait, object-safe) with Claude, Codex, Ollama, and MockProvider
- `ModelRouter` with task-type rules and fallback default
- Typed `AgentEvent` bus: `Dispatcher` trait returns `Pin<Box<dyn Stream>>`, no raw sensitive data on bus
- `PromptRef` stored in shared `Arc<Mutex<PromptStore>>` — resolvable after event emission
- Bounded `TaskQueue` (`tokio::sync::mpsc`) with backpressure
- `WorkerAgent` calling model, emitting events, storing prompts safely
- `OrchestratorAgent` routing and running tasks
- All tests pass with zero real API calls (MockProvider only)
- Smoke test verifies `ModelRequest` and `TaskCompleted` events flow through the full event bus

**Next:** Plan 2 — memory crate (markdown store + LanceDB RAG) + skills crate
