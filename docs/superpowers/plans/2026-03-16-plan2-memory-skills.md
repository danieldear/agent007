# agent007 Plan 2: Memory + Skills

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `memory` crate (markdown file store, RAG pipeline via LanceDB, Indexer, Retriever) and the `skills` crate (skill file loader, Tera template executor) so agents can retrieve persistent context and execute reusable prompt workflows.

**Architecture:** `memory` is a pure library crate with no CLI concerns — it exposes `MemoryStore`/`ScopedMemoryStore` for flat-file reads/writes and a `VectorDB` trait backed by `LanceDBStore` for semantic search; `Indexer` and `Retriever` wire these together with the `EmbeddingProvider` trait from `agent007-models`. `skills` depends on both `agent007-models` and `agent007-memory`, loads `.md` skill files with YAML frontmatter, and `SkillExecutor` renders Tera templates with RAG context + memory values before calling a `ModelProvider`.

**Tech Stack:** lancedb (confirm + pin version pre-build), arrow-array, arrow-schema, futures, serde_yaml, tera, tempfile (dev), async-trait, thiserror, tracing, tokio, chrono

**Prerequisites:** Plan 1 complete (workspace root `Cargo.toml` exists; `agent007-models` crate with `ModelProvider`, `EmbeddingProvider`, `ModelError` published in workspace).

**Spec:** `docs/superpowers/specs/2026-03-16-agent007-design.md`

---

## Chunk 1: memory crate

### File Structure (Chunk 1)

```
crates/memory/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── error.rs          # MemoryError
    ├── store.rs          # MemoryStore, ScopedMemoryStore
    ├── vectordb/
    │   ├── mod.rs        # VectorDB trait + SearchResult
    │   └── lancedb.rs    # LanceDBStore
    ├── indexer.rs        # Indexer
    └── retriever.rs      # Retriever
```

---

### Task 1: memory crate bootstrap + MemoryStore

**Files:**
- Modify: `Cargo.toml` (workspace root) — add `crates/memory` to members, add shared deps
- Create: `crates/memory/Cargo.toml`
- Create: `crates/memory/src/lib.rs`
- Create: `crates/memory/src/error.rs`
- Create: `crates/memory/src/store.rs`

- [ ] **Step 1: Add `crates/memory` to workspace members and shared deps**

In workspace root `Cargo.toml`, add `"crates/memory"` to `[workspace] members` and add to `[workspace.dependencies]`:

```toml
serde_yaml = "0.9"
tempfile = "3"
```

- [ ] **Step 2: Create `crates/memory/Cargo.toml`**

```toml
[package]
name = "agent007-memory"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-models = { path = "../models" }
async-trait = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
tokio = { workspace = true }
futures = { workspace = true }
# PRE-BUILD: confirm exact lancedb + arrow versions on crates.io before building.
# Run `cargo search lancedb` and `cargo tree -p agent007-memory` to validate no arrow conflicts.
lancedb = "0.19"
arrow-array = "52"
arrow-schema = "52"

[dev-dependencies]
tempfile = { workspace = true }
tokio = { workspace = true }
```

- [ ] **Step 3: Create `crates/memory/src/lib.rs`**

Declare all modules and re-export public types. Sketch (no bodies):

```rust
// pub mod error;
// pub mod store;
// pub mod vectordb;
// pub mod indexer;
// pub mod retriever;
//
// pub use error::MemoryError;
// pub use store::{MemoryStore, ScopedMemoryStore};
// pub use vectordb::{VectorDB, SearchResult};
// pub use indexer::Indexer;
// pub use retriever::Retriever;
```

Also create empty stub files so `lib.rs` compiles before each module is implemented:

```bash
mkdir -p crates/memory/src/vectordb
touch crates/memory/src/vectordb/mod.rs
touch crates/memory/src/vectordb/lancedb.rs
touch crates/memory/src/indexer.rs
touch crates/memory/src/retriever.rs
```

- [ ] **Step 4: Create `crates/memory/src/error.rs`**

Variant names + field names only, no method bodies:

```rust
// #[derive(Debug, Error)]
// pub enum MemoryError {
//     #[error("I/O error at {path}: {source}")]
//     Io { path: PathBuf, #[source] source: std::io::Error },
//     #[error("YAML parse error: {0}")]
//     Yaml(#[from] serde_yaml::Error),
//     #[error("JSON error: {0}")]
//     Json(#[from] serde_json::Error),
//     #[error("VectorDB error: {0}")]
//     VectorDb(String),
//     #[error("Embedding error: {0}")]
//     Embedding(String),
// }
```

- [ ] **Step 5: Write failing tests for MemoryStore in `store.rs`**

Add only the `#[cfg(test)]` module to `store.rs`. Tests verify:
- `write("key", "value")` then `read("key")` returns `Some("value")`
- `read("missing")` returns `Ok(None)`
- `scoped("ns").write("k", "v")` writes to `<dir>/ns/k.md` — `global().read("k")` returns `None` (they are independent namespaces)

Use `tempfile::TempDir` for the base directory in each test.

- [ ] **Step 6: Run to confirm compile error (MemoryStore not defined)**

```bash
cargo test -p agent007-memory store::tests 2>&1 | head -20
```

Expected: compile error — `MemoryStore` not yet defined.

- [ ] **Step 7: Implement `crates/memory/src/store.rs`**

Type sketches — field names and method signatures only, no bodies:

```rust
// pub struct MemoryStore {
//     base_dir: PathBuf,
// }
//
// impl MemoryStore {
//     pub fn new(base_dir: impl Into<PathBuf>) -> Self
//     pub fn read(&self, key: &str) -> Result<Option<String>, MemoryError>
//     pub fn write(&self, key: &str, value: &str) -> Result<(), MemoryError>
//     pub fn scoped(self: &Arc<Self>, namespace: &str) -> ScopedMemoryStore
//     pub fn global(self: &Arc<Self>) -> ScopedMemoryStore
//         // calls self.scoped("")
// }
//
// pub struct ScopedMemoryStore {
//     inner: Arc<MemoryStore>,
//     namespace: String,
// }
//
// impl ScopedMemoryStore {
//     pub fn read(&self, key: &str) -> Result<Option<String>, MemoryError>
//     pub fn write(&self, key: &str, value: &str) -> Result<(), MemoryError>
// }
```

Implementation notes:
- `MemoryStore::read/write` operate on `<base_dir>/<key>.md` (creates parent dirs on write)
- `ScopedMemoryStore::read/write` prefix key: namespace `"libp2p"` + key `"user"` → `<base_dir>/libp2p/user.md`; empty namespace `""` → `<base_dir>/user.md` (same as global)

- [ ] **Step 8: Run tests**

```bash
cargo test -p agent007-memory store::tests
```

Expected: 3 tests pass.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml crates/memory/
git commit -m "feat(memory): bootstrap memory crate with MemoryStore and ScopedMemoryStore"
```

---

### Task 2: VectorDB trait + LanceDB implementation

**Files:**
- Create: `crates/memory/src/vectordb/mod.rs`
- Create: `crates/memory/src/vectordb/lancedb.rs`

- [ ] **Step 1: PRE-BUILD — confirm lancedb + arrow versions**

```bash
cargo search lancedb 2>&1 | head -5
cargo search arrow-array 2>&1 | head -5
```

Pin exact versions in `crates/memory/Cargo.toml`. After the first `cargo build -p agent007-memory`, run:

```bash
cargo tree -p agent007-memory | grep -E "arrow|lance"
```

Confirm no duplicate `arrow-array` or `arrow-schema` versions. Resolve any conflicts before proceeding to implementation.

- [ ] **Step 2: Create `crates/memory/src/vectordb/mod.rs`**

Type sketches only:

```rust
// #[async_trait]
// pub trait VectorDB: Send + Sync {
//     async fn upsert(&self, id: &str, vector: Vec<f32>, payload: serde_json::Value)
//         -> Result<(), MemoryError>;
//     async fn search(&self, query: Vec<f32>, limit: usize)
//         -> Result<Vec<SearchResult>, MemoryError>;
// }
//
// #[derive(Debug, Clone)]
// pub struct SearchResult {
//     pub id: String,
//     pub score: f32,          // 1.0 / (1.0 + l2_distance)
//     pub payload: serde_json::Value,
// }
```

- [ ] **Step 3: Write failing test for LanceDB in `vectordb/lancedb.rs`**

Add only the `#[cfg(test)]` module. Test should:
- Create a `LanceDBStore` using a `tempdir` path, table name `"test"`, dims = 4
- Upsert two vectors: `[1.0, 0.0, 0.0, 0.0]` (id `"a"`) and `[0.0, 1.0, 0.0, 0.0]` (id `"b"`)
- Search with query `[1.0, 0.0, 0.0, 0.0]`, limit 1
- Assert `results[0].id == "a"`

- [ ] **Step 4: Run to confirm compile error**

```bash
cargo test -p agent007-memory vectordb::lancedb::tests 2>&1 | head -20
```

Expected: compile error — `LanceDBStore` not defined.

- [ ] **Step 5: Implement `crates/memory/src/vectordb/lancedb.rs`**

Type sketches — field names and method signatures only, no bodies:

```rust
// pub struct LanceDBStore {
//     connection: lancedb::Connection,
//     table_name: String,
//     dims: usize,
// }
//
// impl LanceDBStore {
//     pub async fn new(db_path: &str, table_name: &str, dims: usize)
//         -> Result<Self, MemoryError>
//     // connects/opens db; creates table with schema
//     // [id: Utf8, vector: FixedSizeList<Float32>(dims), payload: Utf8]
//     // only if the table does not already exist
// }
//
// #[async_trait]
// impl VectorDB for LanceDBStore {
//     async fn upsert(&self, id: &str, vector: Vec<f32>, payload: serde_json::Value)
//         -> Result<(), MemoryError>
//     // delete existing row with matching id, then add new RecordBatch
//
//     async fn search(&self, query: Vec<f32>, limit: usize)
//         -> Result<Vec<SearchResult>, MemoryError>
//     // table.vector_search(query).limit(limit).execute()
//     // read result columns by name ("id", "payload", "_distance") NOT by index
//     // score = 1.0 / (1.0 + distance)
// }
```

Implementation note: lancedb appends a `_distance` column to search results — its position in the schema is not guaranteed. Always read columns by name, never by positional index.

- [ ] **Step 6: Run tests**

```bash
cargo test -p agent007-memory vectordb::lancedb::tests
```

Expected: 1 test passes.

- [ ] **Step 7: Commit**

```bash
git add crates/memory/src/vectordb/
git commit -m "feat(memory): add VectorDB trait and LanceDBStore with column-name-safe search"
```

---

### Task 3: Indexer

**Files:**
- Create: `crates/memory/src/indexer.rs`

- [ ] **Step 1: Write failing test for Indexer**

Add only the `#[cfg(test)]` module to `indexer.rs`. Hand-write two test doubles (no `mockall`):
- `MockEmbeddingProvider`: implements `EmbeddingProvider`; returns a fixed `Vec<f32>` (e.g., `vec![0.1; 4]`) for any input
- `MockVectorDB`: implements `VectorDB`; records all `upsert` calls in `Mutex<Vec<(String, Vec<f32>, serde_json::Value)>>`

Test should:
- Create an `Indexer` with `chunk_size = 20`
- Call `indexer.index_text("doc1", "word1 word2 word3 word4 word5").await`
- Assert `mock_db.upsert_calls()` is non-empty
- Assert at least one call has an id starting with `"doc1#"`
- Assert that call's payload contains `"doc_id": "doc1"`

- [ ] **Step 2: Run to confirm compile error**

```bash
cargo test -p agent007-memory indexer::tests 2>&1 | head -20
```

Expected: compile error — `Indexer` not defined.

- [ ] **Step 3: Implement `crates/memory/src/indexer.rs`**

Type sketches only:

```rust
// pub struct Indexer {
//     embedder: Arc<dyn EmbeddingProvider>,
//     db: Arc<dyn VectorDB>,
//     chunk_size: usize,
// }
//
// impl Indexer {
//     pub fn new(
//         embedder: Arc<dyn EmbeddingProvider>,
//         db: Arc<dyn VectorDB>,
//         chunk_size: usize,
//     ) -> Self
//
//     pub async fn index_text(&self, doc_id: &str, text: &str)
//         -> Result<(), MemoryError>
//     // 1. split text into chunks of ~chunk_size chars at whitespace boundaries
//     // 2. for each chunk at index N:
//     //    a. embed chunk: self.embedder.embed(chunk).await
//     //       map_err → MemoryError::Embedding
//     //    b. payload = json!({ "doc_id": doc_id, "chunk_index": N, "text": chunk })
//     //    c. upsert id = format!("{}#{}", doc_id, N)
// }
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007-memory indexer::tests
```

Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/memory/src/indexer.rs
git commit -m "feat(memory): add Indexer — chunk text, embed, upsert into VectorDB"
```

---

### Task 4: Retriever

**Files:**
- Create: `crates/memory/src/retriever.rs`

- [ ] **Step 1: Write failing test for Retriever**

Add only the `#[cfg(test)]` module to `retriever.rs`. Hand-write:
- `MockEmbeddingProvider`: same as in Task 3 (can be a shared test helper or duplicated)
- `MockVectorDB`: `search` always returns a fixed `Vec<SearchResult>` with two entries, each carrying `payload["text"]` set to distinct strings (e.g., `"fragment_alpha"` and `"fragment_beta"`)

Test should:
- Create `Retriever::new(embedder, db, top_k = 2)`
- Call `retriever.retrieve("query").await`
- Assert returned `String` contains `"fragment_alpha"` and `"fragment_beta"`

- [ ] **Step 2: Run to confirm compile error**

```bash
cargo test -p agent007-memory retriever::tests 2>&1 | head -20
```

Expected: compile error — `Retriever` not defined.

- [ ] **Step 3: Implement `crates/memory/src/retriever.rs`**

Type sketches only:

```rust
// pub struct Retriever {
//     embedder: Arc<dyn EmbeddingProvider>,
//     db: Arc<dyn VectorDB>,
//     top_k: usize,
// }
//
// impl Retriever {
//     pub fn new(
//         embedder: Arc<dyn EmbeddingProvider>,
//         db: Arc<dyn VectorDB>,
//         top_k: usize,
//     ) -> Self
//
//     pub async fn retrieve(&self, query: &str) -> Result<String, MemoryError>
//     // 1. embed query: self.embedder.embed(query).await → map_err → MemoryError::Embedding
//     // 2. search: self.db.search(embedding, self.top_k).await
//     // 3. for each SearchResult, extract payload["text"].as_str().unwrap_or("")
//     // 4. join fragments with "\n\n" and return
// }
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007-memory retriever::tests
```

Expected: 1 test passes.

- [ ] **Step 5: Run full memory crate tests**

```bash
cargo test -p agent007-memory
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/memory/src/retriever.rs
git commit -m "feat(memory): add Retriever — embed query, semantic search, return context string"
```

---

## Chunk 2: skills crate

### File Structure (Chunk 2)

```
crates/skills/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── error.rs          # SkillError
    ├── types.rs          # SkillFrontmatter, Skill
    ├── loader.rs         # SkillLoader
    └── executor.rs       # SkillExecutor
```

---

### Task 5: skills crate bootstrap + SkillLoader

**Files:**
- Modify: `Cargo.toml` (workspace root) — add `crates/skills` to members, add `tera` dep
- Create: `crates/skills/Cargo.toml`
- Create: `crates/skills/src/lib.rs`
- Create: `crates/skills/src/error.rs`
- Create: `crates/skills/src/types.rs`
- Create: `crates/skills/src/loader.rs`

- [ ] **Step 1: Add `crates/skills` to workspace members and shared deps**

In workspace root `Cargo.toml`, add `"crates/skills"` to `[workspace] members` and add to `[workspace.dependencies]`:

```toml
tera = "1"
```

- [ ] **Step 2: Create `crates/skills/Cargo.toml`**

```toml
[package]
name = "agent007-skills"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-models = { path = "../models" }
agent007-memory = { path = "../memory" }
async-trait = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = { workspace = true }
tera = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
tokio = { workspace = true }
chrono = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
tokio = { workspace = true }
```

- [ ] **Step 3: Create `crates/skills/src/lib.rs`**

```rust
// pub mod error;
// pub mod types;
// pub mod loader;
// pub mod executor;
//
// pub use error::SkillError;
// pub use types::{SkillFrontmatter, Skill};
// pub use loader::SkillLoader;
// pub use executor::SkillExecutor;
```

Also create empty stub files so `lib.rs` compiles before each module is implemented:

```bash
mkdir -p crates/skills/src
touch crates/skills/src/error.rs crates/skills/src/types.rs \
      crates/skills/src/loader.rs crates/skills/src/executor.rs
```

- [ ] **Step 4: Create `crates/skills/src/error.rs`**

Variant names and field names only, no bodies:

```rust
// #[derive(Debug, Error)]
// pub enum SkillError {
//     #[error("I/O error at {path}: {source}")]
//     Io { path: PathBuf, #[source] source: std::io::Error },
//     #[error("Missing frontmatter in skill file: {path}")]
//     MissingFrontmatter { path: PathBuf },
//     #[error("Frontmatter parse error in {path}: {source}")]
//     FrontmatterParse { path: PathBuf, #[source] source: serde_yaml::Error },
//     #[error("Template render error in skill '{name}': {source}")]
//     TemplateRender { name: String, #[source] source: tera::Error },
//     #[error("Model error in skill '{name}': {source}")]
//     Model { name: String, #[source] source: ModelError },
//     #[error("Memory error in skill '{name}': {source}")]
//     Memory { name: String, #[source] source: MemoryError },
// }
```

- [ ] **Step 5: Create `crates/skills/src/types.rs`**

Type sketches only:

```rust
// #[derive(Debug, Clone, Deserialize)]
// pub struct SkillFrontmatter {
//     pub name: String,
//     pub description: String,
//     pub trigger: String,
//     pub model: String,
// }
//
// #[derive(Debug, Clone)]
// pub struct Skill {
//     pub frontmatter: SkillFrontmatter,
//     pub template: String,
// }
//
// impl Skill {
//     pub fn name(&self) -> &str          // &self.frontmatter.name
//     pub fn trigger(&self) -> &str       // &self.frontmatter.trigger
//     pub fn model(&self) -> &str         // &self.frontmatter.model
//     pub fn template(&self) -> &str      // &self.template
// }
```

- [ ] **Step 6: Write failing tests for SkillLoader in `loader.rs`**

Add only the `#[cfg(test)]` module to `loader.rs`. Tests should:

Test 1 — valid skill file:
- Write a `.md` file to a `tempdir` with content:
  ```
  ---
  name: test-skill
  description: A test skill
  trigger: /test
  model: claude
  ---
  Do something with {{args}}.
  ```
- Call `SkillLoader::new(dir).load_all()?`
- Assert `skills.len() == 1`
- Assert `skills[0].name() == "test-skill"`
- Assert `skills[0].trigger() == "/test"`
- Assert `skills[0].template()` contains `"Do something with"`

Test 2 — non-.md files ignored:
- Same dir, also write a `notes.txt` file
- Assert `load_all()` returns exactly 1 skill

- [ ] **Step 7: Run to confirm compile error**

```bash
cargo test -p agent007-skills loader::tests 2>&1 | head -20
```

Expected: compile error — `SkillLoader` not defined.

- [ ] **Step 8: Implement `crates/skills/src/loader.rs`**

Type sketches only:

```rust
// pub struct SkillLoader {
//     skills_dir: PathBuf,
// }
//
// impl SkillLoader {
//     pub fn new(skills_dir: impl Into<PathBuf>) -> Self
//
//     pub fn load_all(&self) -> Result<Vec<Skill>, SkillError>
//     // 1. read_dir(self.skills_dir)
//     // 2. filter entries whose extension == "md"
//     // 3. for each .md file:
//     //    a. read_to_string
//     //    b. splitn(3, "---") — yields ["", frontmatter_yaml, template_body]
//     //       fewer than 3 parts → Err(SkillError::MissingFrontmatter { path })
//     //    c. serde_yaml::from_str::<SkillFrontmatter>(parts[1])
//     //       map_err → SkillError::FrontmatterParse { path, source }
//     //    d. template = parts[2].trim().to_string()
//     //    e. collect Skill { frontmatter, template }
// }
```

Implementation note: skill files begin with `---\n` so `splitn(3, "---")` yields `["", " frontmatter\n", " body\n"]`. Use `splitn` (not `split`) to avoid splitting on `---` inside the template body itself.

- [ ] **Step 9: Run tests**

```bash
cargo test -p agent007-skills loader::tests
```

Expected: 2 tests pass.

- [ ] **Step 10: Commit**

```bash
git add crates/skills/
git commit -m "feat(skills): bootstrap skills crate with SkillFrontmatter, Skill, SkillLoader"
```

---

### Task 6: SkillExecutor

**Files:**
- Create: `crates/skills/src/executor.rs`

- [ ] **Step 1: Write failing tests for SkillExecutor in `executor.rs`**

Add only the `#[cfg(test)]` module to `executor.rs`. Hand-write test doubles (no `mockall`):
- `MockModelProvider`: implements `ModelProvider`; stores `Arc<AtomicUsize>` call counter; records last `request.model` in `Mutex<Option<String>>`; returns fixed response `"mock-output"`
- `MockRetriever`: a newtype wrapping a fixed `String`; `retrieve(_)` always returns that string

Tests:

Test 1 — response is returned:
- Build a `Skill` with template `"User: {{memory.user}} RAG: {{rag_context}} Args: {{args}}"`
- Pre-write `memory.write("user", "Alice")`
- Call `executor.execute(&skill, "hello").await`
- Assert result `== "mock-output"`

Test 2 — model called exactly once:
- Same setup; assert `mock_provider.call_count() == 1`

Test 3 — correct model name used:
- Build a `Skill` with `frontmatter.model = "ollama"`
- Call `executor.execute(&skill, "x").await`
- Assert `mock_provider.last_model() == Some("ollama")`

- [ ] **Step 2: Run to confirm compile error**

```bash
cargo test -p agent007-skills executor::tests 2>&1 | head -20
```

Expected: compile error — `SkillExecutor` not defined.

- [ ] **Step 3: Implement `crates/skills/src/executor.rs`**

Type sketches only:

```rust
// pub struct SkillExecutor {
//     provider: Arc<dyn ModelProvider>,
//     retriever: Arc<Retriever>,
//     memory: ScopedMemoryStore,
// }
//
// impl SkillExecutor {
//     pub fn new(
//         provider: Arc<dyn ModelProvider>,
//         retriever: Arc<Retriever>,
//         memory: ScopedMemoryStore,
//     ) -> Self
//
//     pub async fn execute(&self, skill: &Skill, args: &str)
//         -> Result<String, SkillError>
//     // 1. rag_context = self.retriever.retrieve(args).await
//     //    map_err → SkillError::Memory { name: skill.name(), source }
//     // 2. memory_user    = self.memory.read("user")?.unwrap_or_default()
//     // 3. memory_project = self.memory.read("project")?.unwrap_or_default()
//     // 4. build tera::Context:
//     //      insert "args"        → args
//     //      insert "rag_context" → rag_context
//     //      insert "memory"      → json!({ "user": memory_user, "project": memory_project })
//     //      insert "date"        → chrono::Utc::now().format("%Y-%m-%d").to_string()
//     // 5. rendered = Tera::one_off(skill.template(), &context, false)
//     //    map_err → SkillError::TemplateRender { name: skill.name(), source }
//     // 6. request = CompletionRequest {
//     //      model: skill.model().to_string(),
//     //      messages: [Message { role: User, content: rendered }],
//     //      max_tokens: None, temperature: None, system: None,
//     //    }
//     // 7. response = self.provider.complete(request).await
//     //    map_err → SkillError::Model { name: skill.name(), source }
//     // 8. Ok(response.content)
// }
```

Implementation notes:
- Use `Tera::one_off(template, &context, false)` — `autoescape = false` prevents HTML-escaping memory content
- Template variables `{{memory.user}}` / `{{memory.project}}` work because the `memory` context value is a JSON object; Tera resolves dot access on JSON objects
- `date` is ISO 8601 date only (`%Y-%m-%d`), matching the spec's `{{date}}` variable

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent007-skills executor::tests
```

Expected: 3 tests pass.

- [ ] **Step 5: Run full skills crate tests**

```bash
cargo test -p agent007-skills
```

Expected: all tests pass.

- [ ] **Step 6: Run full workspace build to catch any dep conflicts**

```bash
cargo build --workspace 2>&1 | head -40
```

If arrow/lancedb dep conflicts appear:
- Run `cargo tree -p agent007-memory | grep -E "arrow|lance"` to identify duplicates
- Pin the conflicting dep version in `crates/memory/Cargo.toml` to match what lancedb requires transitively
- Re-run `cargo build --workspace` until clean

- [ ] **Step 7: Commit**

```bash
git add crates/skills/src/executor.rs
git commit -m "feat(skills): add SkillExecutor — RAG + memory + Tera template + model dispatch"
```

---

## Review Result: ✅ Approved

**Reviewed against:** spec `docs/superpowers/specs/2026-03-16-agent007-design.md` and Plan 1 style patterns.

**Checks:**

- **File paths exact:** All 13 source files and both `Cargo.toml` files are named exactly as in the File Structure blocks. Workspace root path is `Cargo.toml` (correct for this repo layout).
- **Crate dependencies complete:** `agent007-memory` lists all needed deps (agent007-models, async-trait, serde, serde_json, serde_yaml, thiserror, tracing, uuid, tokio, futures, lancedb, arrow-array, arrow-schema; dev: tempfile, tokio). `agent007-skills` lists all needed deps (agent007-models, agent007-memory, async-trait, serde, serde_json, serde_yaml, tera, thiserror, tracing, tokio, chrono; dev: tempfile, tokio). Workspace deps added: serde_yaml, tempfile, tera.
- **TDD order correct:** Every task follows write-failing-test → run-confirm-error → implement → run-pass → commit. No implementation precedes its test.
- **Type sketches consistent with spec:** `MemoryStore.scoped(&Arc<Self>)`/`.global()`, `ScopedMemoryStore { inner: Arc<MemoryStore>, namespace: String }`, `VectorDB` trait signature, `SearchResult` fields, `SkillFrontmatter` fields, `SkillExecutor` template variables (`{{memory.user}}`, `{{memory.project}}`, `{{rag_context}}`, `{{args}}`, `{{date}}`) all match the spec exactly.
- **No missing tasks:** Stub file creation added in Task 1 Step 3 (memory) and Task 5 Step 3 (skills) so `lib.rs` compiles before modules are filled in, matching Plan 1 style. LanceDB PRE-BUILD version check included. Arrow dep conflict resolution documented.
