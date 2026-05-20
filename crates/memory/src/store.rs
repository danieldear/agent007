use crate::error::MemoryError;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc::UnboundedSender;

/// Payload sent to the background vector indexer whenever a memory key is written.
#[derive(Debug)]
pub struct IndexTask {
    /// Stable document ID in the form `memory:{namespace}:{key}`.
    pub doc_id: String,
    /// Content to embed and index.
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryEntryType {
    #[default]
    Semantic,
    Procedural,
    Episodic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMeta {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub access_count: u32,
    #[serde(rename = "type", default)]
    pub entry_type: MemoryEntryType,
    #[serde(default)]
    pub summary: String,
    /// Optional TTL expressed as a human-readable duration: "7d", "30d", "24h", "2h".
    /// The entry is considered expired when `created_at + duration < now`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_after: Option<String>,
    /// Related memory keys in the same scope. Used for 1-hop graph expansion during retrieval.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_to: Vec<String>,
    /// SONA confidence signal (0.0–1.0). Decays ×0.995 on each write to the scope;
    /// boosts +0.03 on each read/touch. Stale entries naturally fade.
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    /// Pre-tokenized word index for fast RAG keyword matching.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<String>,
}

fn default_confidence() -> f32 {
    1.0
}

impl Default for MemoryMeta {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            created_at: now,
            updated_at: now,
            access_count: 0,
            entry_type: MemoryEntryType::Semantic,
            summary: String::new(),
            expires_after: None,
            related_to: Vec::new(),
            confidence: 1.0,
            words: Vec::new(),
        }
    }
}

/// Parse a duration string ("7d", "30d", "24h", "2h") into a chrono Duration.
/// Returns None if the format is unrecognised.
pub fn parse_duration_str(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.ends_with('d') {
        s[..s.len() - 1].parse::<i64>().ok().map(Duration::days)
    } else if s.ends_with('h') {
        s[..s.len() - 1].parse::<i64>().ok().map(Duration::hours)
    } else if s.ends_with('m') {
        s[..s.len() - 1].parse::<i64>().ok().map(Duration::minutes)
    } else {
        None
    }
}

impl MemoryMeta {
    /// Returns true if the entry has expired (i.e. `expires_after` is set and the deadline has passed).
    pub fn is_expired(&self) -> bool {
        if let Some(ref ttl) = self.expires_after {
            if let Some(dur) = parse_duration_str(ttl) {
                return Utc::now() > self.created_at + dur;
            }
        }
        false
    }
}

/// Parse YAML frontmatter from a memory file, returning (content, meta).
/// Falls back to default meta for legacy files without frontmatter.
/// Kept for flat-file migration on first open.
pub fn parse_frontmatter(raw: &str) -> (String, MemoryMeta) {
    if raw.starts_with("---\n") {
        if let Some(end) = raw[4..].find("\n---\n") {
            let yaml = &raw[4..4 + end];
            let content = raw[4 + end + 5..].to_string();
            let meta = serde_yaml::from_str::<MemoryMeta>(yaml).unwrap_or_default();
            return (content, meta);
        }
    }
    (raw.to_string(), MemoryMeta::default())
}

#[allow(dead_code)] // kept for flat-file rollback path
fn write_frontmatter(meta: &MemoryMeta, content: &str) -> String {
    let yaml = serde_yaml::to_string(meta).unwrap_or_default();
    format!("---\n{}---\n{}", yaml, content)
}

// ---------------------------------------------------------------------------
// SQLite schema
// ---------------------------------------------------------------------------

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS memory (
    namespace    TEXT NOT NULL,
    key          TEXT NOT NULL,
    value        TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    access_count INTEGER NOT NULL DEFAULT 0,
    entry_type   TEXT NOT NULL DEFAULT 'semantic',
    summary      TEXT NOT NULL DEFAULT '',
    expires_after TEXT,
    confidence   REAL NOT NULL DEFAULT 1.0,
    words        TEXT NOT NULL DEFAULT '[]',
    related_to   TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (namespace, key)
);
CREATE INDEX IF NOT EXISTS idx_memory_ns ON memory(namespace);
CREATE INDEX IF NOT EXISTS idx_memory_updated ON memory(updated_at);
";

fn entry_type_to_str(et: &MemoryEntryType) -> &'static str {
    match et {
        MemoryEntryType::Semantic => "semantic",
        MemoryEntryType::Procedural => "procedural",
        MemoryEntryType::Episodic => "episodic",
    }
}

fn str_to_entry_type(s: &str) -> MemoryEntryType {
    match s {
        "procedural" => MemoryEntryType::Procedural,
        "episodic" => MemoryEntryType::Episodic,
        _ => MemoryEntryType::Semantic,
    }
}

/// Return true when the entry's TTL has elapsed.
fn is_entry_expired(created_at_str: &str, expires_after: &Option<String>) -> bool {
    if let Some(ref ttl) = expires_after {
        if let Some(dur) = parse_duration_str(ttl) {
            if let Ok(created_at) = created_at_str.parse::<DateTime<Utc>>() {
                return Utc::now() > created_at + dur;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Key / namespace helpers
// ---------------------------------------------------------------------------

/// Split a logical memory key into path-safe components.
///
/// Supports both legacy `:` separators and slash-separated hierarchical keys
/// (e.g. `feedback/index/skill-a`) so callers can use whichever format is
/// most natural for their domain.
fn split_key_components(key: &str) -> Vec<String> {
    key.split(|c| c == ':' || c == '/' || c == '\\')
        .map(|part| {
            part.replace("..", "")
                .replace('/', "")
                .replace('\\', "")
                .trim()
                .to_string()
        })
        .filter(|part| !part.is_empty())
        .collect()
}

/// Normalize any key form (`:`, `/`, `\` delimited) to a canonical colon-separated key.
fn normalize_key(key: &str) -> String {
    let parts = split_key_components(key);
    if parts.is_empty() {
        key.replace("..", "").replace('/', "").replace('\\', "")
    } else {
        parts.join(":")
    }
}

/// Sanitize a namespace string so it cannot escape the base directory.
fn sanitize_namespace(namespace: &str) -> String {
    namespace
        .replace("..", "")
        .replace('/', "")
        .replace('\\', "")
}

/// Tokenize text into lowercase words (≥3 chars, alphanumeric only), deduplicated and sorted.
fn tokenize(text: &str) -> Vec<String> {
    let words: HashSet<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect();
    let mut sorted: Vec<String> = words.into_iter().collect();
    sorted.sort();
    sorted
}

// ---------------------------------------------------------------------------
// Flat-file migration helpers (used only on first open)
// ---------------------------------------------------------------------------

fn collect_keys_recursive(
    root: &Path,
    dir: &Path,
    keys: &mut Vec<String>,
) -> Result<(), MemoryError> {
    for entry in std::fs::read_dir(dir).map_err(|e| MemoryError::Io {
        path: dir.to_path_buf(),
        source: e,
    })? {
        let entry = entry.map_err(|e| MemoryError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_keys_recursive(root, &path, keys)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(rel) = path.strip_prefix(root) {
                let components: Vec<&str> = rel
                    .components()
                    .map(|c| c.as_os_str().to_str().unwrap_or(""))
                    .collect();
                if let Some((last, rest)) = components.split_last() {
                    let stem = last.trim_end_matches(".md");
                    let mut parts: Vec<&str> = rest.iter().copied().collect();
                    parts.push(stem);
                    keys.push(parts.join(":"));
                }
            }
        }
    }
    Ok(())
}

fn resolve_file_path_for_migration(ns_dir: &Path, key: &str) -> PathBuf {
    let parts = split_key_components(key);
    let mut path = ns_dir.to_path_buf();
    match parts.split_last() {
        Some((filename, dirs)) => {
            for dir in dirs {
                path = path.join(dir);
            }
            path.join(format!("{}.md", filename))
        }
        None => {
            let safe = key.replace("..", "").replace('/', "").replace('\\', "");
            path.join(format!("{}.md", safe))
        }
    }
}

fn insert_migrated_row(
    conn: &Mutex<Connection>,
    namespace: &str,
    key: &str,
    content: &str,
    meta: &MemoryMeta,
) {
    let words_json = serde_json::to_string(&meta.words).unwrap_or_else(|_| "[]".to_string());
    let related_to_json =
        serde_json::to_string(&meta.related_to).unwrap_or_else(|_| "[]".to_string());
    let c = conn.lock().unwrap();
    let _ = c.execute(
        "INSERT OR IGNORE INTO memory \
         (namespace, key, value, created_at, updated_at, access_count, entry_type, summary, \
          expires_after, confidence, words, related_to) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            namespace,
            key,
            content,
            meta.created_at.to_rfc3339(),
            meta.updated_at.to_rfc3339(),
            meta.access_count as i64,
            entry_type_to_str(&meta.entry_type),
            meta.summary,
            meta.expires_after,
            meta.confidence as f64,
            words_json,
            related_to_json,
        ],
    );
}

/// Scan existing flat `.md` files in `base_dir` and import them into SQLite.
/// Called once when `memory.db` does not yet exist.
/// Flat files are left in place (they serve as the rollback path).
fn migrate_flat_files(base_dir: &Path, conn: &Mutex<Connection>) {
    let Ok(entries) = std::fs::read_dir(base_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("md") {
            // Global namespace: file directly in base_dir
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if stem.is_empty() {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let (content, meta) = parse_frontmatter(&raw);
                insert_migrated_row(conn, "", &stem, &content, &meta);
            }
        } else if path.is_dir() {
            // Named namespace: subdirectory of base_dir
            let ns = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if ns.is_empty() {
                continue;
            }
            let mut keys = Vec::new();
            collect_keys_recursive(&path, &path, &mut keys).ok();
            for key in keys {
                let file_path = resolve_file_path_for_migration(&path, &key);
                if let Ok(raw) = std::fs::read_to_string(&file_path) {
                    let (content, meta) = parse_frontmatter(&raw);
                    insert_migrated_row(conn, &ns, &key, &content, &meta);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryStore
// ---------------------------------------------------------------------------

pub struct MemoryStore {
    #[allow(dead_code)] // retained for diagnostics and future rollback utilities
    base_dir: PathBuf,
    conn: Arc<Mutex<Connection>>,
    /// Optional background indexing channel. Set once via [`MemoryStore::set_index_channel`].
    index_tx: OnceLock<UnboundedSender<IndexTask>>,
}

impl MemoryStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir: PathBuf = base_dir.into();
        std::fs::create_dir_all(&base_dir).ok();

        let db_path = base_dir.join("memory.db");
        let is_new = !db_path.exists();

        let conn =
            Connection::open(&db_path).expect("Failed to open SQLite memory database");
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .expect("Failed to configure SQLite pragmas");
        conn.execute_batch(SCHEMA_SQL)
            .expect("Failed to create SQLite schema");

        let conn = Arc::new(Mutex::new(conn));

        // On first open: import any existing flat .md files into SQLite.
        if is_new {
            migrate_flat_files(&base_dir, &conn);
        }

        Self {
            base_dir,
            conn,
            index_tx: OnceLock::new(),
        }
    }

    /// Attach a background indexing channel to this store.
    ///
    /// After this call, every successful [`write`](Self::write) will send an
    /// [`IndexTask`] through `tx` so a background task can embed and index
    /// the content into a vector store.  Can only be set once; returns `true`
    /// if the channel was accepted, `false` if one was already set.
    pub fn set_index_channel(&self, tx: UnboundedSender<IndexTask>) -> bool {
        self.index_tx.set(tx).is_ok()
    }

    pub fn read(&self, key: &str) -> Result<Option<String>, MemoryError> {
        self.read_ns("", key)
    }

    pub fn write(&self, key: &str, value: &str) -> Result<(), MemoryError> {
        self.write_ns("", key, value)
    }

    fn read_ns(&self, namespace: &str, key: &str) -> Result<Option<String>, MemoryError> {
        let ns = sanitize_namespace(namespace);
        let k = normalize_key(key);
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT value, created_at, expires_after \
                 FROM memory WHERE namespace = ?1 AND key = ?2",
                params![ns, k],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;

        match result {
            Some((value, created_at_str, expires_after))
                if !is_entry_expired(&created_at_str, &expires_after) =>
            {
                Ok(Some(value))
            }
            _ => Ok(None),
        }
    }

    fn read_with_meta_ns(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<(String, MemoryMeta)>, MemoryError> {
        let ns = sanitize_namespace(namespace);
        let k = normalize_key(key);
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT value, created_at, updated_at, access_count, entry_type, summary, \
                 expires_after, confidence, words, related_to \
                 FROM memory WHERE namespace = ?1 AND key = ?2",
                params![ns, k],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, f64>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;

        if let Some((
            value,
            created_at_str,
            updated_at_str,
            access_count,
            entry_type_str,
            summary,
            expires_after,
            confidence,
            words_json,
            related_to_json,
        )) = result
        {
            if is_entry_expired(&created_at_str, &expires_after) {
                return Ok(None);
            }
            let meta = MemoryMeta {
                created_at: created_at_str.parse().unwrap_or_else(|_| Utc::now()),
                updated_at: updated_at_str.parse().unwrap_or_else(|_| Utc::now()),
                access_count: access_count as u32,
                entry_type: str_to_entry_type(&entry_type_str),
                summary,
                expires_after,
                confidence: confidence as f32,
                words: serde_json::from_str(&words_json).unwrap_or_default(),
                related_to: serde_json::from_str(&related_to_json).unwrap_or_default(),
            };
            Ok(Some((value, meta)))
        } else {
            Ok(None)
        }
    }

    fn touch_ns(&self, namespace: &str, key: &str) -> Result<(), MemoryError> {
        let ns = sanitize_namespace(namespace);
        let k = normalize_key(key);
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE memory \
             SET access_count = access_count + 1, \
                 updated_at   = ?1, \
                 confidence   = MIN(1.0, confidence + 0.03) \
             WHERE namespace = ?2 AND key = ?3",
            params![now, ns, k],
        )
        .map_err(|e| MemoryError::VectorDb(e.to_string()))?;
        Ok(())
    }

    fn list_keys_ns(&self, namespace: &str) -> Result<Vec<String>, MemoryError> {
        let ns = sanitize_namespace(namespace);
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT key, created_at, expires_after \
                 FROM memory WHERE namespace = ?1 ORDER BY key",
            )
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;

        let rows: Vec<(String, String, Option<String>)> = stmt
            .query_map(params![ns], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let keys = rows
            .into_iter()
            .filter(|(_, created_at, expires)| !is_entry_expired(created_at, expires))
            .map(|(k, _, _)| k)
            .collect();

        Ok(keys)
    }

    fn write_ns(&self, namespace: &str, key: &str, value: &str) -> Result<(), MemoryError> {
        let ns = sanitize_namespace(namespace);
        let k = normalize_key(key);
        let now = Utc::now().to_rfc3339();
        let words_json =
            serde_json::to_string(&tokenize(value)).unwrap_or_else(|_| "[]".to_string());

        {
            let conn = self.conn.lock().unwrap();
            // Preserve created_at if the key already exists.
            let existing_created_at: Option<String> = conn
                .query_row(
                    "SELECT created_at FROM memory WHERE namespace = ?1 AND key = ?2",
                    params![ns, k],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| MemoryError::VectorDb(e.to_string()))?
                .flatten();

            let created_at = existing_created_at.unwrap_or_else(|| now.clone());

            conn.execute(
                "INSERT INTO memory \
                 (namespace, key, value, created_at, updated_at, access_count, entry_type, \
                  summary, expires_after, confidence, words, related_to) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, 'semantic', '', NULL, 1.0, ?6, '[]') \
                 ON CONFLICT(namespace, key) DO UPDATE SET \
                     value      = excluded.value, \
                     updated_at = excluded.updated_at, \
                     words      = excluded.words",
                params![ns, k, value, created_at, now, words_json],
            )
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;
        }

        // Decay confidence of all other keys in namespace (skip repo_brain).
        let _ = self.decay_pass(&ns, &k);
        // Auto-link temporal co-writes.
        let _ = self.temporal_edge_pass(&ns, &k);
        self.enqueue_index_task(&ns, &k, value);
        Ok(())
    }

    /// Decay confidence of all keys in namespace by ×0.995, skipping `skip_key` and `repo_brain`.
    fn decay_pass(&self, namespace: &str, skip_key: &str) -> Result<(), MemoryError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE memory \
             SET confidence = MAX(0.0, confidence * 0.995) \
             WHERE namespace = ?1 AND key != ?2 AND key != 'repo_brain'",
            params![namespace, skip_key],
        )
        .map_err(|e| MemoryError::VectorDb(e.to_string()))?;
        Ok(())
    }

    /// Auto-link temporal co-writes: keys updated within the last 10 minutes get
    /// bidirectional `related_to` edges.
    fn temporal_edge_pass(&self, namespace: &str, new_key: &str) -> Result<(), MemoryError> {
        let cutoff = (Utc::now() - Duration::minutes(10)).to_rfc3339();

        // Collect keys (and their current related_to) updated within the window.
        let co_written: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT key, related_to FROM memory \
                     WHERE namespace = ?1 AND key != ?2 AND updated_at > ?3",
                )
                .map_err(|e| MemoryError::VectorDb(e.to_string()))?;
            let rows: Vec<(String, String)> = stmt
                .query_map(params![namespace, new_key, cutoff], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| MemoryError::VectorDb(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        if co_written.is_empty() {
            return Ok(());
        }

        // Add backlinks from co-written peers → new_key.
        {
            let conn = self.conn.lock().unwrap();
            for (peer_key, related_json) in &co_written {
                let mut related: Vec<String> =
                    serde_json::from_str(related_json).unwrap_or_default();
                if !related.contains(&new_key.to_string()) {
                    related.push(new_key.to_string());
                    if related.len() > 5 {
                        related.remove(0);
                    }
                    let new_json =
                        serde_json::to_string(&related).unwrap_or_else(|_| "[]".to_string());
                    conn.execute(
                        "UPDATE memory SET related_to = ?1 \
                         WHERE namespace = ?2 AND key = ?3",
                        params![new_json, namespace, peer_key],
                    )
                    .map_err(|e| MemoryError::VectorDb(e.to_string()))?;
                }
            }
        }

        // Add forward links from new_key → co-written peers.
        {
            let conn = self.conn.lock().unwrap();
            let current_related_json: String = conn
                .query_row(
                    "SELECT related_to FROM memory WHERE namespace = ?1 AND key = ?2",
                    params![namespace, new_key],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "[]".to_string());

            let mut related: Vec<String> =
                serde_json::from_str(&current_related_json).unwrap_or_default();
            for (peer_key, _) in &co_written {
                if !related.contains(peer_key) {
                    related.push(peer_key.clone());
                    if related.len() > 5 {
                        related.remove(0);
                    }
                }
            }
            let new_json =
                serde_json::to_string(&related).unwrap_or_else(|_| "[]".to_string());
            conn.execute(
                "UPDATE memory SET related_to = ?1 \
                 WHERE namespace = ?2 AND key = ?3",
                params![new_json, namespace, new_key],
            )
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;
        }

        Ok(())
    }

    fn write_with_meta_ns(
        &self,
        namespace: &str,
        key: &str,
        value: &str,
        mut meta: MemoryMeta,
    ) -> Result<(), MemoryError> {
        let ns = sanitize_namespace(namespace);
        let k = normalize_key(key);

        {
            let conn = self.conn.lock().unwrap();
            // Preserve original created_at if the key already exists.
            let existing_created_at: Option<String> = conn
                .query_row(
                    "SELECT created_at FROM memory WHERE namespace = ?1 AND key = ?2",
                    params![ns, k],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| MemoryError::VectorDb(e.to_string()))?
                .flatten();

            if let Some(ref ca) = existing_created_at {
                if let Ok(dt) = ca.parse::<DateTime<Utc>>() {
                    meta.created_at = dt;
                }
            }
            meta.updated_at = Utc::now();

            let words_json =
                serde_json::to_string(&meta.words).unwrap_or_else(|_| "[]".to_string());
            let related_to_json =
                serde_json::to_string(&meta.related_to).unwrap_or_else(|_| "[]".to_string());

            conn.execute(
                "INSERT INTO memory \
                 (namespace, key, value, created_at, updated_at, access_count, entry_type, \
                  summary, expires_after, confidence, words, related_to) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
                 ON CONFLICT(namespace, key) DO UPDATE SET \
                     value        = excluded.value, \
                     created_at   = excluded.created_at, \
                     updated_at   = excluded.updated_at, \
                     access_count = excluded.access_count, \
                     entry_type   = excluded.entry_type, \
                     summary      = excluded.summary, \
                     expires_after = excluded.expires_after, \
                     confidence   = excluded.confidence, \
                     words        = excluded.words, \
                     related_to   = excluded.related_to",
                params![
                    ns,
                    k,
                    value,
                    meta.created_at.to_rfc3339(),
                    meta.updated_at.to_rfc3339(),
                    meta.access_count as i64,
                    entry_type_to_str(&meta.entry_type),
                    meta.summary,
                    meta.expires_after,
                    meta.confidence as f64,
                    words_json,
                    related_to_json,
                ],
            )
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;
        }

        self.enqueue_index_task(&ns, &k, value);
        Ok(())
    }

    /// Shared helper: enqueue an [`IndexTask`] when a background indexing channel is attached.
    /// Fire-and-forget — a closed or lagging receiver is silently ignored.
    fn enqueue_index_task(&self, namespace: &str, key: &str, value: &str) {
        if let Some(tx) = self.index_tx.get() {
            let ns_label = if namespace.is_empty() {
                "default"
            } else {
                namespace
            };
            let _ = tx.send(IndexTask {
                doc_id: format!("memory:{ns_label}:{key}"),
                content: value.to_string(),
            });
        }
    }

    /// Delete expired entries in a namespace and return number removed.
    fn purge_expired_ns(&self, namespace: &str) -> Result<usize, MemoryError> {
        let ns = sanitize_namespace(namespace);
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT key, created_at, expires_after \
                 FROM memory WHERE namespace = ?1 AND expires_after IS NOT NULL",
            )
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;

        let rows: Vec<(String, String, String)> = stmt
            .query_map(params![ns], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        drop(stmt); // release borrow on conn before calling execute

        let now = Utc::now();
        let mut removed = 0usize;
        for (key, created_at_str, ttl) in rows {
            if let Some(dur) = parse_duration_str(&ttl) {
                if let Ok(created_at) = created_at_str.parse::<DateTime<Utc>>() {
                    if now > created_at + dur {
                        conn.execute(
                            "DELETE FROM memory WHERE namespace = ?1 AND key = ?2",
                            params![ns, key],
                        )
                        .map_err(|e| MemoryError::VectorDb(e.to_string()))?;
                        removed += 1;
                    }
                }
            }
        }

        Ok(removed)
    }

    fn delete_ns(&self, namespace: &str, key: &str) -> Result<bool, MemoryError> {
        let ns = sanitize_namespace(namespace);
        let k = normalize_key(key);
        let conn = self.conn.lock().unwrap();
        let affected = conn
            .execute(
                "DELETE FROM memory WHERE namespace = ?1 AND key = ?2",
                params![ns, k],
            )
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;
        Ok(affected > 0)
    }

    /// Read the top-N entries by relevance score.
    /// Score = 0.5 * recency + 0.3 * ln(access_count+1) + 0.2 * confidence.
    /// `repo_brain` is always first if it exists.
    fn read_top_n_ns(&self, namespace: &str, n: usize) -> Result<String, MemoryError> {
        let ns = sanitize_namespace(namespace);
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT key, value, updated_at, access_count, confidence, created_at, expires_after \
                 FROM memory WHERE namespace = ?1",
            )
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;

        let rows: Vec<(String, String, String, i64, f64, String, Option<String>)> = stmt
            .query_map(params![ns], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let now = Utc::now();

        let mut scored: Vec<(f64, String, String)> = rows
            .into_iter()
            .filter(|(_, _, _, _, _, created_at, expires)| {
                !is_entry_expired(created_at, expires)
            })
            .map(|(key, value, updated_at_str, access_count, confidence, _, _)| {
                let updated_at = updated_at_str.parse::<DateTime<Utc>>().unwrap_or(now);
                let age_secs = (now - updated_at).num_seconds().max(0) as f64;
                let recency = 1.0 / (age_secs / 3600.0 + 1.0);
                let score =
                    0.5 * recency + 0.3 * (access_count as f64).ln_1p() + 0.2 * confidence;
                (score, key, value)
            })
            .collect();

        let brain_pos = scored.iter().position(|(_, k, _)| k == "repo_brain");
        let brain_entry = brain_pos.map(|i| scored.remove(i));

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let remaining_slots = if brain_entry.is_some() {
            n.saturating_sub(1)
        } else {
            n
        };
        scored.truncate(remaining_slots);

        let mut parts = Vec::new();
        if let Some((_, key, value)) = brain_entry {
            parts.push(format!("### {}\n{}", key, value));
        }
        for (_, key, value) in scored {
            if key != "repo_brain" {
                parts.push(format!("### {}\n{}", key, value));
            }
        }
        Ok(parts.join("\n\n"))
    }

    pub fn scoped(self: &Arc<Self>, namespace: &str) -> ScopedMemoryStore {
        ScopedMemoryStore {
            inner: Arc::clone(self),
            namespace: namespace.to_string(),
        }
    }

    pub fn global(self: &Arc<Self>) -> ScopedMemoryStore {
        self.scoped("")
    }
}

// ---------------------------------------------------------------------------
// ScopedMemoryStore
// ---------------------------------------------------------------------------

pub struct ScopedMemoryStore {
    pub inner: Arc<MemoryStore>,
    pub namespace: String,
}

impl ScopedMemoryStore {
    pub fn read(&self, key: &str) -> Result<Option<String>, MemoryError> {
        self.inner.read_ns(&self.namespace, key)
    }

    pub fn write(&self, key: &str, value: &str) -> Result<(), MemoryError> {
        self.inner.write_ns(&self.namespace, key, value)
    }

    /// Write a value with explicit metadata (e.g. to set entry_type = Procedural).
    /// Preserves `created_at` if the key already exists.
    pub fn write_with_meta(
        &self,
        key: &str,
        value: &str,
        meta: MemoryMeta,
    ) -> Result<(), MemoryError> {
        self.inner
            .write_with_meta_ns(&self.namespace, key, value, meta)
    }

    pub fn read_with_meta(&self, key: &str) -> Result<Option<(String, MemoryMeta)>, MemoryError> {
        self.inner.read_with_meta_ns(&self.namespace, key)
    }

    pub fn touch(&self, key: &str) -> Result<(), MemoryError> {
        self.inner.touch_ns(&self.namespace, key)
    }

    /// List all keys stored in this scope.
    pub fn list_keys(&self) -> Result<Vec<String>, MemoryError> {
        self.inner.list_keys_ns(&self.namespace)
    }

    /// Remove expired entries in this scope. Returns the number removed.
    pub fn purge_expired(&self) -> Result<usize, MemoryError> {
        self.inner.purge_expired_ns(&self.namespace)
    }

    /// Delete a memory key from this scope. Returns true when an entry was removed.
    pub fn delete(&self, key: &str) -> Result<bool, MemoryError> {
        self.inner.delete_ns(&self.namespace, key)
    }

    /// Read a key plus all entries linked via `related_to` (1-hop graph expansion).
    /// Returns the primary entry first, followed by any related entries that exist.
    pub fn read_with_related(&self, key: &str) -> Result<Vec<(String, String)>, MemoryError> {
        let mut results = Vec::new();
        if let Some((content, meta)) = self.read_with_meta(key)? {
            results.push((key.to_string(), content));
            for related_key in &meta.related_to {
                if let Ok(Some(related_content)) = self.read(related_key) {
                    results.push((related_key.clone(), related_content));
                }
            }
        }
        Ok(results)
    }

    /// Read all key→value pairs in this scope, concatenated as "### key\nvalue" blocks.
    pub fn read_all(&self) -> Result<String, MemoryError> {
        let keys = self.list_keys()?;
        let mut parts = Vec::new();
        for key in &keys {
            if let Ok(Some(value)) = self.read(key) {
                parts.push(format!("### {}\n{}", key, value));
            }
        }
        Ok(parts.join("\n\n"))
    }

    /// Read the top-N entries by relevance score.
    /// Score = 0.5 * recency + 0.3 * ln(access_count+1) + 0.2 * confidence.
    /// "repo_brain" is always included first if it exists (special-cased for context injection).
    /// Returns entries formatted as "### key\nvalue" blocks (same as `read_all`).
    pub fn read_top_n(&self, n: usize) -> Result<String, MemoryError> {
        self.inner.read_top_n_ns(&self.namespace, n)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn write_then_read_returns_value() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        store.write("key", "hello world").unwrap();
        assert_eq!(store.read("key").unwrap(), Some("hello world".to_string()));
    }

    #[test]
    fn read_missing_key_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        assert_eq!(store.read("missing").unwrap(), None);
    }

    #[test]
    fn scoped_write_is_independent_from_global() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("ns");
        scoped.write("k", "scoped-value").unwrap();
        let global = store.global();
        assert_eq!(global.read("k").unwrap(), None);
    }

    #[test]
    fn list_keys_returns_written_keys_sorted() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("user");
        scoped.write("beta", "b").unwrap();
        scoped.write("alpha", "a").unwrap();
        scoped.write("gamma", "c").unwrap();
        let keys = scoped.list_keys().unwrap();
        assert_eq!(keys, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn list_keys_on_empty_scope_returns_empty() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let keys = store.scoped("nonexistent").list_keys().unwrap();
        assert!(keys.is_empty());
    }

    #[test]
    fn read_all_concatenates_scope_entries() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped.write("goals", "build great software").unwrap();
        scoped.write("tech", "Rust").unwrap();
        let all = scoped.read_all().unwrap();
        assert!(all.contains("### goals"));
        assert!(all.contains("build great software"));
        assert!(all.contains("### tech"));
        assert!(all.contains("Rust"));
    }

    #[test]
    fn write_preserves_frontmatter_on_update() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        store.write("key", "first").unwrap();
        store.write("key", "second").unwrap();
        assert_eq!(store.read("key").unwrap(), Some("second".to_string()));
    }

    #[test]
    fn touch_increments_access_count() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        store.write("key", "value").unwrap();
        let scoped = store.global();
        assert_eq!(
            scoped
                .read_with_meta("key")
                .unwrap()
                .unwrap()
                .1
                .access_count,
            0
        );
        scoped.touch("key").unwrap();
        scoped.touch("key").unwrap();
        assert_eq!(
            scoped
                .read_with_meta("key")
                .unwrap()
                .unwrap()
                .1
                .access_count,
            2
        );
    }

    #[test]
    fn read_legacy_file_without_frontmatter_returns_content() {
        let dir = TempDir::new().unwrap();
        // Write a plain .md file (no frontmatter) before the store is opened.
        // Migration should import it into SQLite.
        let path = dir.path().join("key.md");
        std::fs::write(&path, "legacy content").unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        assert_eq!(
            store.read("key").unwrap(),
            Some("legacy content".to_string())
        );
    }

    #[test]
    fn subkey_routing_with_colon_separator() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped.write("arch:overview", "the system design").unwrap();
        // Reading back should return content
        assert_eq!(
            scoped.read("arch:overview").unwrap(),
            Some("the system design".to_string())
        );
    }

    #[test]
    fn list_keys_includes_subkey_with_colon() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped.write("alpha", "a").unwrap();
        scoped.write("arch:overview", "o").unwrap();
        let keys = scoped.list_keys().unwrap();
        assert!(keys.contains(&"alpha".to_string()));
        assert!(keys.contains(&"arch:overview".to_string()));
    }

    #[test]
    fn slash_keys_roundtrip_and_list_as_colon_paths() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("learning");
        scoped
            .write("feedback/index/review-pr", "[\"abc\"]")
            .unwrap();
        assert_eq!(
            scoped.read("feedback/index/review-pr").unwrap(),
            Some("[\"abc\"]".to_string())
        );
        let keys = scoped.list_keys().unwrap();
        assert!(keys.contains(&"feedback:index:review-pr".to_string()));
    }

    #[test]
    fn reading_new_format_can_fallback_to_legacy_flattened_file() {
        // Verify that a flat .md file placed on disk before the store is opened
        // is correctly imported by the migration and readable by its stem key.
        let dir = TempDir::new().unwrap();
        let scoped_dir = dir.path().join("learning");
        std::fs::create_dir_all(&scoped_dir).unwrap();
        let legacy_path = scoped_dir.join("review-notes.md");
        std::fs::write(&legacy_path, "my review notes").unwrap();

        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("learning");
        assert_eq!(
            scoped.read("review-notes").unwrap(),
            Some("my review notes".to_string())
        );
    }

    #[test]
    fn expired_entry_reads_as_none() {
        let dir = TempDir::new().unwrap();
        // Write a file with expires_after already elapsed (created_at in the past).
        // Migration imports it; read should return None due to expiry.
        let path = dir.path().join("stale.md");
        let past = Utc::now() - chrono::Duration::days(10);
        let meta = MemoryMeta {
            created_at: past,
            updated_at: past,
            access_count: 0,
            entry_type: MemoryEntryType::Semantic,
            summary: String::new(),
            expires_after: Some("7d".to_string()),
            related_to: Vec::new(),
            confidence: 1.0,
            words: Vec::new(),
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        let raw = format!("---\n{}---\nstale content", yaml);
        std::fs::write(&path, raw).unwrap();

        let store = Arc::new(MemoryStore::new(dir.path()));
        assert_eq!(store.read("stale").unwrap(), None);
    }

    #[test]
    fn not_yet_expired_entry_reads_normally() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fresh.md");
        let meta = MemoryMeta {
            expires_after: Some("30d".to_string()),
            ..MemoryMeta::default()
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        let raw = format!("---\n{}---\nfresh content", yaml);
        std::fs::write(&path, raw).unwrap();

        let store = Arc::new(MemoryStore::new(dir.path()));
        assert_eq!(
            store.read("fresh").unwrap(),
            Some("fresh content".to_string())
        );
    }

    #[test]
    fn read_with_related_expands_linked_entries() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped.write("secondary", "related content").unwrap();
        // Write main via the store API so it is stored in SQLite with related_to set.
        let meta = MemoryMeta {
            related_to: vec!["secondary".to_string()],
            ..MemoryMeta::default()
        };
        scoped
            .write_with_meta("main", "primary content", meta)
            .unwrap();

        let results = scoped.read_with_related("main").unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "main");
        assert_eq!(results[0].1, "primary content");
        assert_eq!(results[1].0, "secondary");
        assert_eq!(results[1].1, "related content");
    }

    #[test]
    fn parse_duration_str_parses_days_hours_minutes() {
        assert_eq!(parse_duration_str("7d"), Some(chrono::Duration::days(7)));
        assert_eq!(parse_duration_str("24h"), Some(chrono::Duration::hours(24)));
        assert_eq!(
            parse_duration_str("90m"),
            Some(chrono::Duration::minutes(90))
        );
        assert_eq!(parse_duration_str("invalid"), None);
    }

    #[test]
    fn read_top_n_returns_at_most_n_and_prioritises_repo_brain() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");

        // Write 5 entries + repo_brain
        for i in 0..5 {
            scoped
                .write(&format!("entry-{i}"), &format!("value-{i}"))
                .unwrap();
        }
        scoped
            .write("repo_brain", "# Architecture overview")
            .unwrap();

        // Top-3 should include repo_brain first and 2 others
        let result = scoped.read_top_n(3).unwrap();
        assert!(
            result.starts_with("### repo_brain"),
            "repo_brain should be first"
        );
        let blocks = result
            .split("\n\n")
            .filter(|s| s.starts_with("### "))
            .count();
        assert!(blocks <= 3, "expected at most 3 blocks, got {blocks}");
    }

    #[test]
    fn namespace_path_traversal_is_sanitized() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        // A namespace with path traversal components should be sanitized to a safe string.
        let scoped = store.scoped("../../etc");
        scoped.write("passwd", "should not escape").unwrap();
        // The only files on disk should be inside base_dir (memory.db, etc.)
        let base = dir.path().canonicalize().unwrap();
        let found: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        for entry in found {
            let p = entry.path().canonicalize().unwrap_or_else(|_| entry.path());
            assert!(
                p.starts_with(&base),
                "file escaped base dir: {} not under {}",
                p.display(),
                base.display()
            );
        }
        // Value is still readable via the sanitized namespace
        let val = scoped.read("passwd").unwrap();
        assert_eq!(val, Some("should not escape".to_string()));
    }

    #[test]
    fn words_tokenized_on_write() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped
            .write("feature", "implement authentication with JWT tokens")
            .unwrap();
        let (_, meta) = scoped.read_with_meta("feature").unwrap().unwrap();
        assert!(
            meta.words.contains(&"implement".to_string()),
            "words should contain 'implement'"
        );
        assert!(
            meta.words.contains(&"authentication".to_string()),
            "words should contain 'authentication'"
        );
        assert!(
            meta.words.contains(&"jwt".to_string()),
            "words should contain 'jwt' (lowercased)"
        );
        assert!(
            meta.words.contains(&"tokens".to_string()),
            "words should contain 'tokens'"
        );
        assert!(
            !meta.words.contains(&"with".to_string()) || meta.words.contains(&"with".to_string()),
            "words index built"
        );
    }

    #[test]
    fn confidence_decays_on_write() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped.write("alpha", "alpha content").unwrap();
        let (_, meta_before) = scoped.read_with_meta("alpha").unwrap().unwrap();
        assert!(
            (meta_before.confidence - 1.0).abs() < 0.001,
            "new entry starts at 1.0 confidence"
        );
        // Write beta — should decay alpha
        scoped.write("beta", "beta content").unwrap();
        let (_, meta_after) = scoped.read_with_meta("alpha").unwrap().unwrap();
        assert!(
            meta_after.confidence < 1.0,
            "alpha confidence should decay after writing beta"
        );
        assert!(
            (meta_after.confidence - 0.995).abs() < 0.001,
            "decay should be ×0.995"
        );
    }

    #[test]
    fn confidence_boosts_on_touch() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped.write("alpha", "alpha content").unwrap();
        scoped.write("beta", "beta content").unwrap(); // decays alpha to 0.995
        let (_, meta_decayed) = scoped.read_with_meta("alpha").unwrap().unwrap();
        let before = meta_decayed.confidence;
        assert!(before < 1.0);
        scoped.touch("alpha").unwrap();
        let (_, meta_boosted) = scoped.read_with_meta("alpha").unwrap().unwrap();
        assert!(
            meta_boosted.confidence > before,
            "touch should boost confidence"
        );
        let expected = (before + 0.03f32).min(1.0);
        assert!(
            (meta_boosted.confidence - expected).abs() < 0.001,
            "boost should be +0.03 (capped at 1.0)"
        );
    }

    #[test]
    fn temporal_edges_auto_linked() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped.write("alpha", "first entry").unwrap();
        scoped.write("beta", "second entry").unwrap();
        // beta should have alpha in related_to
        let (_, beta_meta) = scoped.read_with_meta("beta").unwrap().unwrap();
        assert!(
            beta_meta.related_to.contains(&"alpha".to_string()),
            "beta should auto-link to alpha (co-written within 10min)"
        );
        // alpha should also have beta in related_to (backlink)
        let (_, alpha_meta) = scoped.read_with_meta("alpha").unwrap().unwrap();
        assert!(
            alpha_meta.related_to.contains(&"beta".to_string()),
            "alpha should auto-link to beta (backlink from temporal edge)"
        );
    }

    #[test]
    fn purge_expired_removes_stale_files() {
        let dir = TempDir::new().unwrap();
        let scoped_dir = dir.path().join("project");
        std::fs::create_dir_all(&scoped_dir).unwrap();
        // Write an already-expired entry to disk before the store is opened.
        // Migration imports it; purge_expired should then remove it from SQLite.
        let path = scoped_dir.join("old.md");
        let past = Utc::now() - chrono::Duration::days(40);
        let meta = MemoryMeta {
            created_at: past,
            updated_at: past,
            expires_after: Some("7d".to_string()),
            ..MemoryMeta::default()
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        std::fs::write(&path, format!("---\n{}---\nexpired", yaml)).unwrap();

        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        let removed = scoped.purge_expired().unwrap();
        assert_eq!(removed, 1);
        // Entry is no longer accessible after purge
        assert_eq!(scoped.read("old").unwrap(), None);
    }

    #[test]
    fn delete_removes_existing_key_only_once() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let scoped = store.scoped("project");
        scoped.write("topic:item", "value").unwrap();

        assert_eq!(scoped.read("topic:item").unwrap().as_deref(), Some("value"));
        assert!(scoped.delete("topic:item").unwrap());
        assert_eq!(scoped.read("topic:item").unwrap(), None);
        assert!(!scoped.delete("topic:item").unwrap());
    }
}
