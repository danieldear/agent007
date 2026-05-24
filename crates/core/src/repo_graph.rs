use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;

const GRAPH_VERSION: u32 = 1;
const GRAPH_FILENAME: &str = "repo_graph_v1.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RepoGraphNodeKind {
    File,
    Symbol,
    Module,
    Doc,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RepoGraphEdgeKind {
    Defines,
    Imports,
    Calls,
    Documents,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoGraphNode {
    pub id: String,
    pub kind: RepoGraphNodeKind,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoGraphEdge {
    pub kind: RepoGraphEdgeKind,
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepoGraphCounts {
    pub files: usize,
    pub rust_files: usize,
    pub doc_files: usize,
    pub symbols: usize,
    pub modules: usize,
    pub docs: usize,
    pub edges: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoGraph {
    pub version: u32,
    pub root: String,
    pub built_at: String,
    pub graph_path: String,
    pub counts: RepoGraphCounts,
    pub nodes: Vec<RepoGraphNode>,
    pub edges: Vec<RepoGraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoGraphStatus {
    pub exists: bool,
    pub graph_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub built_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts: Option<RepoGraphCounts>,
    pub stale: bool,
    pub stale_files: usize,
}

#[derive(Debug, Clone)]
pub struct RepoGraphBuilder {
    root: PathBuf,
}

#[derive(Debug, Clone)]
struct PendingCall {
    from_symbol_id: String,
    target_name: String,
    path: String,
    line: usize,
}

impl RepoGraphBuilder {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn build(&self) -> Result<RepoGraph, CoreError> {
        let root = self
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.root.clone());
        let graph_path = default_graph_path_for_root(&root);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut counts = RepoGraphCounts::default();
        let mut symbol_index: HashMap<String, Vec<String>> = HashMap::new();
        let mut pending_calls = Vec::new();

        // ── Pass 1: Rust files — build symbol_index first so doc links work ──
        let all_files = walk_repo_files(&root)?;
        for path in &all_files {
            let rel = relative_path(&root, path);
            let path_str = rel.to_string_lossy().to_string();
            if is_rust_file(path) {
                counts.rust_files += 1;
                let parsed = parse_rust_file(path, &path_str)?;
                counts.files += 1;
                let file_id = format!("file:{path_str}");
                nodes.push(RepoGraphNode {
                    id: file_id.clone(),
                    kind: RepoGraphNodeKind::File,
                    name: path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&path_str)
                        .to_string(),
                    path: Some(path_str.clone()),
                    language: Some("rust".into()),
                    symbol_kind: None,
                    line: None,
                    signature: None,
                });
                for import_path in parsed.imports {
                    let module_id = format!("module:{import_path}");
                    nodes.push(RepoGraphNode {
                        id: module_id.clone(),
                        kind: RepoGraphNodeKind::Module,
                        name: import_path.clone(),
                        path: None,
                        language: Some("rust".into()),
                        symbol_kind: None,
                        line: None,
                        signature: None,
                    });
                    edges.push(RepoGraphEdge {
                        kind: RepoGraphEdgeKind::Imports,
                        from: file_id.clone(),
                        to: module_id,
                        path: Some(path_str.clone()),
                        line: None,
                    });
                }

                for symbol in parsed.symbols {
                    counts.symbols += 1;
                    let node_id = format!("symbol:{path_str}:{}:{}", symbol.name, symbol.line);
                    nodes.push(RepoGraphNode {
                        id: node_id.clone(),
                        kind: RepoGraphNodeKind::Symbol,
                        name: symbol.name.clone(),
                        path: Some(path_str.clone()),
                        language: Some("rust".into()),
                        symbol_kind: Some(symbol.kind.clone()),
                        line: Some(symbol.line),
                        signature: Some(symbol.signature.clone()),
                    });
                    symbol_index
                        .entry(symbol.name.clone())
                        .or_default()
                        .push(node_id.clone());
                    edges.push(RepoGraphEdge {
                        kind: RepoGraphEdgeKind::Defines,
                        from: file_id.clone(),
                        to: node_id.clone(),
                        path: Some(path_str.clone()),
                        line: Some(symbol.line),
                    });
                    for call in symbol.calls {
                        pending_calls.push(PendingCall {
                            from_symbol_id: node_id.clone(),
                            target_name: call.name,
                            path: path_str.clone(),
                            line: call.line,
                        });
                    }
                }
            }
        }

        // ── Pass 2: Doc files — symbol_index is now fully populated ──────────
        for path in &all_files {
            let rel = relative_path(&root, path);
            let path_str = rel.to_string_lossy().to_string();
            if is_doc_file(path) {
                counts.doc_files += 1;
                counts.files += 1;
                counts.docs += 1;
                let doc_id = format!("doc:{path_str}");
                nodes.push(RepoGraphNode {
                    id: doc_id.clone(),
                    kind: RepoGraphNodeKind::Doc,
                    name: path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&path_str)
                        .to_string(),
                    path: Some(path_str.clone()),
                    language: Some("markdown".into()),
                    symbol_kind: None,
                    line: None,
                    signature: None,
                });
                for symbol_name in extract_doc_symbol_mentions(path)? {
                    if let Some(matches) = symbol_index.get(&symbol_name) {
                        for symbol_id in matches {
                            edges.push(RepoGraphEdge {
                                kind: RepoGraphEdgeKind::Documents,
                                from: doc_id.clone(),
                                to: symbol_id.clone(),
                                path: Some(path_str.clone()),
                                line: None,
                            });
                        }
                    }
                }
            }
        }

        for pending in pending_calls {
            if let Some(targets) = symbol_index.get(&pending.target_name) {
                for target in targets {
                    edges.push(RepoGraphEdge {
                        kind: RepoGraphEdgeKind::Calls,
                        from: pending.from_symbol_id.clone(),
                        to: target.clone(),
                        path: Some(pending.path.clone()),
                        line: Some(pending.line),
                    });
                }
            }
        }

        dedupe_nodes(&mut nodes, &mut counts);
        dedupe_edges(&mut edges);
        counts.edges = edges.len();

        Ok(RepoGraph {
            version: GRAPH_VERSION,
            root: root.display().to_string(),
            built_at: Utc::now().to_rfc3339(),
            graph_path: graph_path.display().to_string(),
            counts,
            nodes,
            edges,
        })
    }
}

#[derive(Debug, Clone)]
struct RustSymbol {
    name: String,
    kind: String,
    line: usize,
    signature: String,
    calls: Vec<RustCall>,
}

#[derive(Debug, Clone)]
struct RustCall {
    name: String,
    line: usize,
}

#[derive(Debug, Clone, Default)]
struct ParsedRustFile {
    imports: Vec<String>,
    symbols: Vec<RustSymbol>,
}

pub fn default_graph_path_for_root(root: &Path) -> PathBuf {
    root.join(".agent007").join("runtime").join(GRAPH_FILENAME)
}

pub fn load_graph(path: &Path) -> Result<RepoGraph, CoreError> {
    let text = fs::read_to_string(path).map_err(|e| CoreError::io(path, e))?;
    serde_json::from_str(&text).map_err(CoreError::from)
}

pub fn save_graph(graph: &RepoGraph, path: &Path) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
    }
    let text = serde_json::to_string_pretty(graph)?;
    fs::write(path, text).map_err(|e| CoreError::io(path, e))
}

pub fn build_and_save_graph(root: &Path, path: Option<&Path>) -> Result<RepoGraph, CoreError> {
    let builder = RepoGraphBuilder::new(root);
    let mut graph = builder.build()?;
    let target = path
        .map(PathBuf::from)
        .unwrap_or_else(|| default_graph_path_for_root(root));
    graph.graph_path = target.display().to_string();
    save_graph(&graph, &target)?;
    Ok(graph)
}

pub fn graph_status(path: &Path) -> RepoGraphStatus {
    if !path.exists() {
        return RepoGraphStatus {
            exists: false,
            graph_path: path.display().to_string(),
            root: None,
            built_at: None,
            version: None,
            counts: None,
            stale: false,
            stale_files: 0,
        };
    }
    let graph = match load_graph(path) {
        Ok(graph) => graph,
        Err(_) => {
            return RepoGraphStatus {
                exists: true,
                graph_path: path.display().to_string(),
                root: None,
                built_at: None,
                version: None,
                counts: None,
                stale: true,
                stale_files: 0,
            };
        }
    };
    let built_at = chrono::DateTime::parse_from_rfc3339(&graph.built_at)
        .ok()
        .map(|dt| dt.with_timezone(&Utc));
    let mut stale_files = 0usize;
    if let Some(built_at) = built_at {
        let root = PathBuf::from(&graph.root);
        for node in &graph.nodes {
            // Check both Rust source files and doc files for staleness; changes
            // to either can invalidate doc→symbol edges or symbol definitions.
            if node.kind != RepoGraphNodeKind::File && node.kind != RepoGraphNodeKind::Doc {
                continue;
            }
            let Some(rel) = &node.path else { continue };
            let p = root.join(rel);
            let Ok(meta) = fs::metadata(&p) else { continue };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            let modified = chrono::DateTime::<Utc>::from(modified);
            if modified > built_at {
                stale_files += 1;
            }
        }
    }
    RepoGraphStatus {
        exists: true,
        graph_path: path.display().to_string(),
        root: Some(graph.root),
        built_at: Some(graph.built_at),
        version: Some(graph.version),
        counts: Some(graph.counts),
        stale: stale_files > 0,
        stale_files,
    }
}

pub fn resolve_graph_path(root: Option<&Path>, graph_path: Option<&Path>) -> PathBuf {
    if let Some(graph_path) = graph_path {
        return graph_path.to_path_buf();
    }
    let base = root
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    default_graph_path_for_root(&base)
}

pub fn load_or_build_graph(root: &Path, graph_path: Option<&Path>) -> Result<RepoGraph, CoreError> {
    let path = resolve_graph_path(Some(root), graph_path);
    if path.exists() {
        return load_graph(&path);
    }
    build_and_save_graph(root, Some(&path))
}

fn dedupe_nodes(nodes: &mut Vec<RepoGraphNode>, counts: &mut RepoGraphCounts) {
    let mut seen = BTreeSet::new();
    nodes.retain(|node| seen.insert(node.id.clone()));
    counts.modules = nodes
        .iter()
        .filter(|node| node.kind == RepoGraphNodeKind::Module)
        .count();
}

fn dedupe_edges(edges: &mut Vec<RepoGraphEdge>) {
    let mut seen = BTreeSet::new();
    edges.retain(|edge| {
        seen.insert((
            edge.kind.clone(),
            edge.from.clone(),
            edge.to.clone(),
            edge.path.clone(),
            edge.line,
        ))
    });
}

fn walk_repo_files(root: &Path) -> Result<Vec<PathBuf>, CoreError> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = fs::read_dir(&dir).map_err(|e| CoreError::io(&dir, e))?;
        for entry in rd {
            let entry = entry.map_err(|e| CoreError::io(&dir, e))?;
            let path = entry.path();
            let ft = entry.file_type().map_err(|e| CoreError::io(&path, e))?;
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if ft.is_symlink() || should_skip_name(name) {
                continue;
            }
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn should_skip_name(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".venv" | "venv" | ".idea" | ".zed" | ".agent007" // skip runtime artifacts (vectordb, sessions, etc.)
    )
}

fn is_rust_file(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()) == Some("rs")
}

fn is_doc_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("md") | Some("mdx")
    )
}

fn relative_path(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

fn parse_rust_file(path: &Path, rel_path: &str) -> Result<ParsedRustFile, CoreError> {
    let text = fs::read_to_string(path).map_err(|e| CoreError::io(path, e))?;
    let import_re = Regex::new(r"^\s*use\s+([^;]+);").expect("valid regex");
    let fn_re = Regex::new(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)")
        .expect("valid regex");
    let type_re = Regex::new(r"^\s*(?:pub\s+)?(struct|enum|trait)\s+([A-Za-z_][A-Za-z0-9_]*)")
        .expect("valid regex");
    let call_re = Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(").expect("valid regex");

    let mut parsed = ParsedRustFile::default();
    let mut current: Option<RustSymbol> = None;
    let mut brace_depth = 0usize;

    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        if let Some(caps) = import_re.captures(line) {
            parsed.imports.push(caps[1].trim().to_string());
        }
        if current.is_none() {
            if let Some(caps) = fn_re.captures(line) {
                let name = caps[1].to_string();
                brace_depth = line
                    .matches('{')
                    .count()
                    .saturating_sub(line.matches('}').count());
                let mut symbol = RustSymbol {
                    name: name.clone(),
                    kind: "function".into(),
                    line: line_no,
                    signature: format!("{rel_path}::{name}"),
                    calls: Vec::new(),
                };
                for cap in call_re.captures_iter(line) {
                    let call_name = cap[1].to_string();
                    if should_skip_call_name(&call_name) || call_name == symbol.name {
                        continue;
                    }
                    symbol.calls.push(RustCall {
                        name: call_name,
                        line: line_no,
                    });
                }
                if brace_depth == 0 {
                    parsed.symbols.push(symbol);
                } else {
                    current = Some(symbol);
                }
                continue;
            }
            if let Some(caps) = type_re.captures(line) {
                let kind = caps[1].to_string();
                let name = caps[2].to_string();
                parsed.symbols.push(RustSymbol {
                    name: name.clone(),
                    kind,
                    line: line_no,
                    signature: format!("{rel_path}::{name}"),
                    calls: Vec::new(),
                });
            }
            continue;
        }

        if let Some(symbol) = current.as_mut() {
            for cap in call_re.captures_iter(line) {
                let call_name = cap[1].to_string();
                if should_skip_call_name(&call_name) || call_name == symbol.name {
                    continue;
                }
                symbol.calls.push(RustCall {
                    name: call_name,
                    line: line_no,
                });
            }
            brace_depth += line.matches('{').count();
            brace_depth = brace_depth.saturating_sub(line.matches('}').count());
            if brace_depth == 0 {
                parsed
                    .symbols
                    .push(current.take().expect("current symbol exists"));
            }
        }
    }

    if let Some(symbol) = current.take() {
        parsed.symbols.push(symbol);
    }
    Ok(parsed)
}

fn should_skip_call_name(name: &str) -> bool {
    matches!(
        name,
        "if" | "for"
            | "while"
            | "loop"
            | "match"
            | "return"
            | "Some"
            | "Ok"
            | "Err"
            | "Self"
            | "assert"
            | "assert_eq"
            | "assert_ne"
            | "format"
            | "println"
            | "eprintln"
            | "vec"
    )
}

fn extract_doc_symbol_mentions(path: &Path) -> Result<Vec<String>, CoreError> {
    let text = fs::read_to_string(path).map_err(|e| CoreError::io(path, e))?;
    let code_span = Regex::new(r"`([A-Za-z_][A-Za-z0-9_]*)`").expect("valid regex");
    let heading = Regex::new(r"^#+\s+([A-Za-z_][A-Za-z0-9_]*)").expect("valid regex");
    let mut names = BTreeSet::new();
    for cap in code_span.captures_iter(&text) {
        names.insert(cap[1].to_string());
    }
    for line in text.lines() {
        if let Some(cap) = heading.captures(line) {
            names.insert(cap[1].to_string());
        }
    }
    Ok(names.into_iter().collect())
}

pub fn symbol_lookup(graph: &RepoGraph, symbol: &str, exact: bool) -> Vec<RepoGraphNode> {
    let needle = symbol.to_lowercase();
    let mut out: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind == RepoGraphNodeKind::Symbol || node.kind == RepoGraphNodeKind::Module
        })
        .filter(|node| {
            let hay = node.name.to_lowercase();
            if exact {
                hay == needle
            } else {
                hay.contains(&needle)
            }
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
    out
}

pub fn callers_for_symbol(
    graph: &RepoGraph,
    symbol: &str,
    exact: bool,
) -> Vec<BTreeMap<String, String>> {
    let targets = symbol_lookup(graph, symbol, exact);
    let target_ids: BTreeSet<_> = targets.iter().map(|node| node.id.clone()).collect();
    if target_ids.is_empty() {
        return Vec::new();
    }

    let node_map: HashMap<_, _> = graph.nodes.iter().map(|n| (n.id.clone(), n)).collect();
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for edge in &graph.edges {
        if edge.kind != RepoGraphEdgeKind::Calls || !target_ids.contains(&edge.to) {
            continue;
        }
        let Some(from_node) = node_map.get(&edge.from) else {
            continue;
        };
        let Some(to_node) = node_map.get(&edge.to) else {
            continue;
        };
        let key = (
            from_node.name.clone(),
            from_node.path.clone().unwrap_or_default(),
            to_node.name.clone(),
            to_node.path.clone().unwrap_or_default(),
            edge.line.unwrap_or(0),
        );
        if !seen.insert(key) {
            continue;
        }
        let mut row = BTreeMap::new();
        row.insert("caller".into(), from_node.name.clone());
        row.insert(
            "caller_path".into(),
            from_node.path.clone().unwrap_or_default(),
        );
        row.insert("callee".into(), to_node.name.clone());
        row.insert(
            "callee_path".into(),
            to_node.path.clone().unwrap_or_default(),
        );
        row.insert(
            "line".into(),
            edge.line.map(|v| v.to_string()).unwrap_or_default(),
        );
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_graph_for_small_rust_repo() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            r#"
use crate::util::helper;

pub fn alpha() {
    helper();
}

fn beta() {
    alpha();
}
"#,
        )
        .unwrap();
        fs::write(dir.path().join("README.md"), "Uses `alpha`.").unwrap();
        let graph = RepoGraphBuilder::new(dir.path()).build().unwrap();
        assert!(graph.nodes.iter().any(|n| n.name == "alpha"));
        assert!(graph
            .edges
            .iter()
            .any(|e| e.kind == RepoGraphEdgeKind::Calls));
        assert!(graph.counts.rust_files >= 1);
    }

    #[test]
    fn callers_lookup_returns_matching_callers() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            r#"
pub fn alpha() {}

pub fn beta() {
    alpha();
}
"#,
        )
        .unwrap();
        let graph = RepoGraphBuilder::new(dir.path()).build().unwrap();
        let callers = callers_for_symbol(&graph, "alpha", true);
        assert!(callers
            .iter()
            .any(|row| row.get("caller") == Some(&"beta".to_string())));
    }
}
