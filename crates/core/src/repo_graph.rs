use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::tree_sitter_support::enrich_parsed_rust_file_with_tree_sitter;

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
    pub missing_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoGraphNeighborhood {
    pub symbol: String,
    pub exact: bool,
    pub max_depth: usize,
    pub matched_symbols: Vec<RepoGraphNode>,
    pub nodes: Vec<RepoGraphNode>,
    pub edges: Vec<RepoGraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoGraphPathResult {
    pub from: String,
    pub to: String,
    pub exact: bool,
    pub found: bool,
    pub steps: Vec<RepoGraphPathStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoGraphPathStep {
    pub from_id: String,
    pub from_name: String,
    pub from_path: Option<String>,
    pub edge_kind: String,
    pub to_id: String,
    pub to_name: String,
    pub to_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoGraphQueryContext {
    pub query: String,
    pub matched_symbols: Vec<RepoGraphNode>,
    pub related_docs: Vec<RepoGraphNode>,
    pub files: Vec<String>,
    pub text: String,
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
pub(crate) struct RustSymbol {
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) line: usize,
    pub(crate) signature: String,
    pub(crate) calls: Vec<RustCall>,
}

#[derive(Debug, Clone)]
pub(crate) struct RustCall {
    pub(crate) name: String,
    pub(crate) line: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedRustFile {
    pub(crate) imports: Vec<String>,
    pub(crate) symbols: Vec<RustSymbol>,
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

pub fn refresh_graph_for_paths(
    root: &Path,
    path: Option<&Path>,
    requested_paths: &[PathBuf],
) -> Result<RepoGraph, CoreError> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let target = path
        .map(PathBuf::from)
        .unwrap_or_else(|| default_graph_path_for_root(&root));
    if requested_paths.is_empty() || !target.exists() {
        return build_and_save_graph(&root, Some(&target));
    }

    let mut graph = load_graph(&target)?;
    let requested = normalize_requested_paths(&root, requested_paths);
    if requested.is_empty() {
        return build_and_save_graph(&root, Some(&target));
    }

    let old_nodes = std::mem::take(&mut graph.nodes);
    let old_edges = std::mem::take(&mut graph.edges);
    let old_node_map: HashMap<_, _> = old_nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect();
    let removed_node_ids: BTreeSet<String> = old_nodes
        .iter()
        .filter(|node| {
            node.path
                .as_ref()
                .map(|path| requested.contains(path))
                .unwrap_or(false)
        })
        .map(|node| node.id.clone())
        .collect();

    let mut rebound_calls: Vec<PendingCall> = Vec::new();
    let mut rebound_docs: Vec<(String, String, String)> = Vec::new();
    let mut retained_edges = Vec::new();

    for edge in old_edges {
        if edge
            .path
            .as_ref()
            .map(|path| requested.contains(path))
            .unwrap_or(false)
        {
            continue;
        }
        let from_removed = removed_node_ids.contains(&edge.from);
        let to_removed = removed_node_ids.contains(&edge.to);

        match edge.kind {
            RepoGraphEdgeKind::Calls if !from_removed && to_removed => {
                if let Some(target_node) = old_node_map.get(&edge.to) {
                    rebound_calls.push(PendingCall {
                        from_symbol_id: edge.from.clone(),
                        target_name: target_node.name.clone(),
                        path: edge.path.clone().unwrap_or_default(),
                        line: edge.line.unwrap_or(0),
                    });
                }
            }
            RepoGraphEdgeKind::Documents if !from_removed && to_removed => {
                if let (Some(doc_node), Some(target_node)) =
                    (old_node_map.get(&edge.from), old_node_map.get(&edge.to))
                {
                    rebound_docs.push((
                        doc_node.id.clone(),
                        doc_node.path.clone().unwrap_or_default(),
                        target_node.name.clone(),
                    ));
                }
            }
            _ => {}
        }

        if from_removed || to_removed {
            continue;
        }
        retained_edges.push(edge);
    }

    let mut nodes: Vec<RepoGraphNode> = old_nodes
        .into_iter()
        .filter(|node| !removed_node_ids.contains(&node.id))
        .collect();
    let mut edges = retained_edges;
    let mut counts = RepoGraphCounts::default();
    let mut symbol_index: HashMap<String, Vec<String>> = build_symbol_index(&nodes);
    let mut pending_calls = Vec::new();
    let mut pending_doc_links = Vec::new();

    for rel_path in &requested {
        let abs_path = root.join(rel_path);
        if !abs_path.exists() {
            continue;
        }
        if is_rust_file(&abs_path) {
            patch_rust_file(
                &abs_path,
                rel_path,
                &mut nodes,
                &mut edges,
                &mut symbol_index,
                &mut pending_calls,
                &mut counts,
            )?;
        } else if is_doc_file(&abs_path) {
            patch_doc_file(
                &abs_path,
                rel_path,
                &mut nodes,
                &mut pending_doc_links,
                &mut counts,
            )?;
        }
    }

    for pending in pending_calls.into_iter().chain(rebound_calls.into_iter()) {
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

    for (doc_id, doc_path, symbol_name) in pending_doc_links
        .into_iter()
        .chain(rebound_docs.into_iter())
    {
        if let Some(targets) = symbol_index.get(&symbol_name) {
            for target in targets {
                edges.push(RepoGraphEdge {
                    kind: RepoGraphEdgeKind::Documents,
                    from: doc_id.clone(),
                    to: target.clone(),
                    path: Some(doc_path.clone()),
                    line: None,
                });
            }
        }
    }

    dedupe_nodes(&mut nodes, &mut counts);
    dedupe_edges(&mut edges);
    counts = recalculate_counts(&nodes, &edges);

    let refreshed = RepoGraph {
        version: GRAPH_VERSION,
        root: root.display().to_string(),
        built_at: Utc::now().to_rfc3339(),
        graph_path: target.display().to_string(),
        counts,
        nodes,
        edges,
    };
    save_graph(&refreshed, &target)?;
    Ok(refreshed)
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
            missing_files: 0,
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
                missing_files: 0,
            };
        }
    };
    let built_at = chrono::DateTime::parse_from_rfc3339(&graph.built_at)
        .ok()
        .map(|dt| dt.with_timezone(&Utc));
    let mut stale_files = 0usize;
    let mut missing_files = 0usize;
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
            let Ok(meta) = fs::metadata(&p) else {
                missing_files += 1;
                continue;
            };
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
        stale: stale_files > 0 || missing_files > 0,
        stale_files,
        missing_files,
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

fn recalculate_counts(nodes: &[RepoGraphNode], edges: &[RepoGraphEdge]) -> RepoGraphCounts {
    RepoGraphCounts {
        files: nodes
            .iter()
            .filter(|node| matches!(node.kind, RepoGraphNodeKind::File | RepoGraphNodeKind::Doc))
            .count(),
        rust_files: nodes
            .iter()
            .filter(|node| {
                node.kind == RepoGraphNodeKind::File && node.language.as_deref() == Some("rust")
            })
            .count(),
        doc_files: nodes
            .iter()
            .filter(|node| node.kind == RepoGraphNodeKind::Doc)
            .count(),
        symbols: nodes
            .iter()
            .filter(|node| node.kind == RepoGraphNodeKind::Symbol)
            .count(),
        modules: nodes
            .iter()
            .filter(|node| node.kind == RepoGraphNodeKind::Module)
            .count(),
        docs: nodes
            .iter()
            .filter(|node| node.kind == RepoGraphNodeKind::Doc)
            .count(),
        edges: edges.len(),
    }
}

fn build_symbol_index(nodes: &[RepoGraphNode]) -> HashMap<String, Vec<String>> {
    let mut symbol_index: HashMap<String, Vec<String>> = HashMap::new();
    for node in nodes {
        if node.kind == RepoGraphNodeKind::Symbol {
            symbol_index
                .entry(node.name.clone())
                .or_default()
                .push(node.id.clone());
        }
    }
    symbol_index
}

fn normalize_requested_paths(root: &Path, requested_paths: &[PathBuf]) -> BTreeSet<String> {
    requested_paths
        .iter()
        .map(|path| {
            if path.is_absolute() {
                relative_path(root, path).to_string_lossy().to_string()
            } else {
                path.to_string_lossy().to_string()
            }
        })
        .collect()
}

fn patch_rust_file(
    abs_path: &Path,
    rel_path: &str,
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    symbol_index: &mut HashMap<String, Vec<String>>,
    pending_calls: &mut Vec<PendingCall>,
    counts: &mut RepoGraphCounts,
) -> Result<(), CoreError> {
    counts.rust_files += 1;
    counts.files += 1;
    let parsed = parse_rust_file(abs_path, rel_path)?;
    let file_id = format!("file:{rel_path}");
    nodes.push(RepoGraphNode {
        id: file_id.clone(),
        kind: RepoGraphNodeKind::File,
        name: abs_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(rel_path)
            .to_string(),
        path: Some(rel_path.to_string()),
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
            path: Some(rel_path.to_string()),
            line: None,
        });
    }
    for symbol in parsed.symbols {
        counts.symbols += 1;
        let node_id = format!("symbol:{rel_path}:{}:{}", symbol.name, symbol.line);
        nodes.push(RepoGraphNode {
            id: node_id.clone(),
            kind: RepoGraphNodeKind::Symbol,
            name: symbol.name.clone(),
            path: Some(rel_path.to_string()),
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
            path: Some(rel_path.to_string()),
            line: Some(symbol.line),
        });
        for call in symbol.calls {
            pending_calls.push(PendingCall {
                from_symbol_id: node_id.clone(),
                target_name: call.name,
                path: rel_path.to_string(),
                line: call.line,
            });
        }
    }
    Ok(())
}

fn patch_doc_file(
    abs_path: &Path,
    rel_path: &str,
    nodes: &mut Vec<RepoGraphNode>,
    pending_doc_links: &mut Vec<(String, String, String)>,
    counts: &mut RepoGraphCounts,
) -> Result<(), CoreError> {
    counts.doc_files += 1;
    counts.files += 1;
    counts.docs += 1;
    let doc_id = format!("doc:{rel_path}");
    nodes.push(RepoGraphNode {
        id: doc_id.clone(),
        kind: RepoGraphNodeKind::Doc,
        name: abs_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(rel_path)
            .to_string(),
        path: Some(rel_path.to_string()),
        language: Some("markdown".into()),
        symbol_kind: None,
        line: None,
        signature: None,
    });
    for symbol_name in extract_doc_symbol_mentions(abs_path)? {
        pending_doc_links.push((doc_id.clone(), rel_path.to_string(), symbol_name));
    }
    Ok(())
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
    let mut parsed = parse_rust_file_fallback(&text, rel_path);
    enrich_parsed_rust_file_with_tree_sitter(&text, rel_path, &mut parsed);
    Ok(parsed)
}

fn parse_rust_file_fallback(text: &str, rel_path: &str) -> ParsedRustFile {
    let import_re = Regex::new(r"^\s*use\s+([^;]+);").expect("valid regex");
    let fn_re = Regex::new(
        r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|unsafe|extern\s+"[^"]+")\s+)*fn\s+([A-Za-z_][A-Za-z0-9_]*)"#,
    )
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
    parsed
}

pub(crate) fn should_skip_call_name(name: &str) -> bool {
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
    let (targets, target_ids) = target_nodes_for_symbol(graph, symbol, exact);
    if target_ids.is_empty() {
        return Vec::new();
    }
    let _ = targets;
    let node_map = graph_node_map(graph);
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
        let key = unique_edge_key(from_node, to_node, edge);
        if !seen.insert(key) {
            continue;
        }
        rows.push(call_row("caller", "callee", from_node, to_node, edge));
    }
    rows
}

pub fn callees_for_symbol(
    graph: &RepoGraph,
    symbol: &str,
    exact: bool,
) -> Vec<BTreeMap<String, String>> {
    let (targets, target_ids) = target_nodes_for_symbol(graph, symbol, exact);
    if target_ids.is_empty() {
        return Vec::new();
    }
    let _ = targets;
    let node_map = graph_node_map(graph);
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for edge in &graph.edges {
        if edge.kind != RepoGraphEdgeKind::Calls || !target_ids.contains(&edge.from) {
            continue;
        }
        let Some(from_node) = node_map.get(&edge.from) else {
            continue;
        };
        let Some(to_node) = node_map.get(&edge.to) else {
            continue;
        };
        let key = unique_edge_key(from_node, to_node, edge);
        if !seen.insert(key) {
            continue;
        }
        rows.push(call_row("caller", "callee", from_node, to_node, edge));
    }
    rows
}

pub fn doc_links_for_symbol(
    graph: &RepoGraph,
    symbol: &str,
    exact: bool,
) -> Vec<BTreeMap<String, String>> {
    let (_, target_ids) = target_nodes_for_symbol(graph, symbol, exact);
    if target_ids.is_empty() {
        return Vec::new();
    }
    let node_map = graph_node_map(graph);
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for edge in &graph.edges {
        if edge.kind != RepoGraphEdgeKind::Documents || !target_ids.contains(&edge.to) {
            continue;
        }
        let Some(doc_node) = node_map.get(&edge.from) else {
            continue;
        };
        let Some(symbol_node) = node_map.get(&edge.to) else {
            continue;
        };
        let key = (
            doc_node.id.clone(),
            symbol_node.id.clone(),
            edge.path.clone(),
            edge.line,
        );
        if !seen.insert(key) {
            continue;
        }
        let mut row = BTreeMap::new();
        row.insert("doc".into(), doc_node.name.clone());
        row.insert("doc_path".into(), doc_node.path.clone().unwrap_or_default());
        row.insert("symbol".into(), symbol_node.name.clone());
        row.insert(
            "symbol_path".into(),
            symbol_node.path.clone().unwrap_or_default(),
        );
        rows.push(row);
    }
    rows
}

pub fn usage_graph_for_symbol(
    graph: &RepoGraph,
    symbol: &str,
    exact: bool,
    max_depth: usize,
) -> RepoGraphNeighborhood {
    let matched_symbols = symbol_lookup(graph, symbol, exact);
    let seed_ids: Vec<String> = matched_symbols.iter().map(|node| node.id.clone()).collect();
    let (nodes, edges) = graph_neighborhood(graph, &seed_ids, max_depth.max(1));
    RepoGraphNeighborhood {
        symbol: symbol.to_string(),
        exact,
        max_depth: max_depth.max(1),
        matched_symbols,
        nodes,
        edges,
    }
}

pub fn impact_radius_for_symbol(
    graph: &RepoGraph,
    symbol: &str,
    exact: bool,
    max_depth: usize,
) -> RepoGraphNeighborhood {
    usage_graph_for_symbol(graph, symbol, exact, max_depth.max(2))
}

pub fn dep_path_between_symbols(
    graph: &RepoGraph,
    from_symbol: &str,
    to_symbol: &str,
    exact: bool,
) -> RepoGraphPathResult {
    let from_matches = symbol_lookup(graph, from_symbol, exact);
    let to_matches = symbol_lookup(graph, to_symbol, exact);
    let from_ids: BTreeSet<_> = from_matches.iter().map(|node| node.id.clone()).collect();
    let to_ids: BTreeSet<_> = to_matches.iter().map(|node| node.id.clone()).collect();
    if from_ids.is_empty() || to_ids.is_empty() {
        return RepoGraphPathResult {
            from: from_symbol.to_string(),
            to: to_symbol.to_string(),
            exact,
            found: false,
            steps: Vec::new(),
        };
    }

    let node_map = graph_node_map(graph);
    let mut adjacency: HashMap<String, Vec<(String, RepoGraphEdge)>> = HashMap::new();
    for edge in &graph.edges {
        adjacency
            .entry(edge.from.clone())
            .or_default()
            .push((edge.to.clone(), edge.clone()));
        adjacency.entry(edge.to.clone()).or_default().push((
            edge.from.clone(),
            RepoGraphEdge {
                kind: edge.kind.clone(),
                from: edge.to.clone(),
                to: edge.from.clone(),
                path: edge.path.clone(),
                line: edge.line,
            },
        ));
    }

    let mut queue = std::collections::VecDeque::new();
    let mut visited = BTreeSet::new();
    let mut prev: HashMap<String, (String, RepoGraphEdge)> = HashMap::new();
    for id in &from_ids {
        queue.push_back(id.clone());
        visited.insert(id.clone());
    }

    let mut found_target: Option<String> = None;
    while let Some(current) = queue.pop_front() {
        if to_ids.contains(&current) {
            found_target = Some(current);
            break;
        }
        for (next, edge) in adjacency.get(&current).cloned().unwrap_or_default() {
            if visited.insert(next.clone()) {
                prev.insert(next.clone(), (current.clone(), edge));
                queue.push_back(next);
            }
        }
    }

    let Some(target_id) = found_target else {
        return RepoGraphPathResult {
            from: from_symbol.to_string(),
            to: to_symbol.to_string(),
            exact,
            found: false,
            steps: Vec::new(),
        };
    };

    let mut steps = Vec::new();
    let mut cursor = target_id.clone();
    while let Some((prev_id, edge)) = prev.get(&cursor).cloned() {
        let Some(from_node) = node_map.get(&prev_id) else {
            break;
        };
        let Some(to_node) = node_map.get(&cursor) else {
            break;
        };
        steps.push(RepoGraphPathStep {
            from_id: prev_id.clone(),
            from_name: from_node.name.clone(),
            from_path: from_node.path.clone(),
            edge_kind: format!("{:?}", edge.kind).to_lowercase(),
            to_id: cursor.clone(),
            to_name: to_node.name.clone(),
            to_path: to_node.path.clone(),
        });
        cursor = prev_id;
    }
    steps.reverse();

    RepoGraphPathResult {
        from: from_symbol.to_string(),
        to: to_symbol.to_string(),
        exact,
        found: true,
        steps,
    }
}

pub fn context_bundle_for_query(
    graph: &RepoGraph,
    query: &str,
    max_symbols: usize,
    max_neighbors: usize,
) -> RepoGraphQueryContext {
    let keywords = extract_query_keywords(query);
    let mut matched_symbols = Vec::new();
    let mut seen_ids = BTreeSet::new();
    for keyword in keywords {
        let mut matches = symbol_lookup(graph, &keyword, true);
        if matches.is_empty() {
            matches = symbol_lookup(graph, &keyword, false);
        }
        for node in matches {
            if seen_ids.insert(node.id.clone()) {
                matched_symbols.push(node);
            }
            if matched_symbols.len() >= max_symbols {
                break;
            }
        }
        if matched_symbols.len() >= max_symbols {
            break;
        }
    }

    let mut files = BTreeSet::new();
    let mut related_docs = Vec::new();
    let mut text = String::new();
    for node in matched_symbols.iter().take(max_symbols) {
        if let Some(path) = &node.path {
            files.insert(path.clone());
        }
        text.push_str(&format!(
            "[symbol] {} ({})\n",
            node.name,
            node.path.clone().unwrap_or_default()
        ));

        for row in callers_for_symbol(graph, &node.name, true)
            .into_iter()
            .take(max_neighbors)
        {
            text.push_str(&format!(
                "  caller: {} ({})\n",
                row.get("caller").cloned().unwrap_or_default(),
                row.get("caller_path").cloned().unwrap_or_default()
            ));
            if let Some(path) = row.get("caller_path") {
                if !path.is_empty() {
                    files.insert(path.clone());
                }
            }
        }

        for row in callees_for_symbol(graph, &node.name, true)
            .into_iter()
            .take(max_neighbors)
        {
            text.push_str(&format!(
                "  callee: {} ({})\n",
                row.get("callee").cloned().unwrap_or_default(),
                row.get("callee_path").cloned().unwrap_or_default()
            ));
            if let Some(path) = row.get("callee_path") {
                if !path.is_empty() {
                    files.insert(path.clone());
                }
            }
        }

        for row in doc_links_for_symbol(graph, &node.name, true)
            .into_iter()
            .take(max_neighbors)
        {
            let doc_path = row.get("doc_path").cloned().unwrap_or_default();
            text.push_str(&format!(
                "  doc: {} ({})\n",
                row.get("doc").cloned().unwrap_or_default(),
                doc_path
            ));
            if !doc_path.is_empty() {
                files.insert(doc_path.clone());
                if let Some(doc_node) = graph
                    .nodes
                    .iter()
                    .find(|candidate| candidate.path.as_deref() == Some(doc_path.as_str()))
                {
                    related_docs.push(doc_node.clone());
                }
            }
        }
        text.push('\n');
    }

    RepoGraphQueryContext {
        query: query.to_string(),
        matched_symbols,
        related_docs,
        files: files.into_iter().collect(),
        text: text.trim().to_string(),
    }
}

pub fn evidence_refs_for_text(
    root: &Path,
    graph_path: Option<&Path>,
    text: &str,
    limit: usize,
) -> Vec<String> {
    let Ok(graph) = load_or_build_graph(root, graph_path) else {
        return Vec::new();
    };
    let bundle = context_bundle_for_query(&graph, text, limit.max(1), 1);
    bundle
        .matched_symbols
        .into_iter()
        .take(limit)
        .map(|node| node.id)
        .collect()
}

fn target_nodes_for_symbol(
    graph: &RepoGraph,
    symbol: &str,
    exact: bool,
) -> (Vec<RepoGraphNode>, BTreeSet<String>) {
    let targets = symbol_lookup(graph, symbol, exact);
    let target_ids = targets.iter().map(|node| node.id.clone()).collect();
    (targets, target_ids)
}

fn graph_node_map(graph: &RepoGraph) -> HashMap<String, &RepoGraphNode> {
    graph.nodes.iter().map(|n| (n.id.clone(), n)).collect()
}

fn unique_edge_key(
    from_node: &RepoGraphNode,
    to_node: &RepoGraphNode,
    edge: &RepoGraphEdge,
) -> (String, String, String, String, usize) {
    (
        from_node.name.clone(),
        from_node.path.clone().unwrap_or_default(),
        to_node.name.clone(),
        to_node.path.clone().unwrap_or_default(),
        edge.line.unwrap_or(0),
    )
}

fn call_row(
    from_label: &str,
    to_label: &str,
    from_node: &RepoGraphNode,
    to_node: &RepoGraphNode,
    edge: &RepoGraphEdge,
) -> BTreeMap<String, String> {
    let mut row = BTreeMap::new();
    row.insert(from_label.into(), from_node.name.clone());
    row.insert(
        format!("{from_label}_path"),
        from_node.path.clone().unwrap_or_default(),
    );
    row.insert(to_label.into(), to_node.name.clone());
    row.insert(
        format!("{to_label}_path"),
        to_node.path.clone().unwrap_or_default(),
    );
    row.insert(
        "line".into(),
        edge.line.map(|v| v.to_string()).unwrap_or_default(),
    );
    row
}

fn graph_neighborhood(
    graph: &RepoGraph,
    seed_ids: &[String],
    max_depth: usize,
) -> (Vec<RepoGraphNode>, Vec<RepoGraphEdge>) {
    let node_map = graph_node_map(graph);
    let mut queue = std::collections::VecDeque::new();
    let mut seen = BTreeSet::new();
    let mut edge_seen = BTreeSet::new();
    let mut included_edges = Vec::new();
    for id in seed_ids {
        queue.push_back((id.clone(), 0usize));
        seen.insert(id.clone());
    }
    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        for edge in &graph.edges {
            let next = if edge.from == current {
                Some(edge.to.clone())
            } else if edge.to == current {
                Some(edge.from.clone())
            } else {
                None
            };
            let Some(next) = next else { continue };
            let edge_key = (
                edge.kind.clone(),
                edge.from.clone(),
                edge.to.clone(),
                edge.path.clone(),
                edge.line,
            );
            if edge_seen.insert(edge_key) {
                included_edges.push(edge.clone());
            }
            if seen.insert(next.clone()) {
                queue.push_back((next, depth + 1));
            }
        }
    }
    let mut nodes: Vec<RepoGraphNode> = seen
        .into_iter()
        .filter_map(|id| node_map.get(&id).cloned().cloned())
        .collect();
    nodes.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
    included_edges.sort_by(|a, b| a.from.cmp(&b.from).then(a.to.cmp(&b.to)));
    (nodes, included_edges)
}

fn extract_query_keywords(query: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for token in query
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
        .filter(|token| token.len() >= 3)
    {
        out.insert(token.to_lowercase());
    }
    out.into_iter().collect()
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

    #[test]
    fn graph_queries_cover_callees_paths_docs_and_context() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::create_dir_all(dir.path().join("docs")).unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            r#"
pub fn alpha() {}
pub fn beta() { alpha(); }
pub fn gamma() { beta(); }
"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("docs/alpha.md"),
            "Alpha references `alpha` and explains the symbol.\n",
        )
        .unwrap();

        let graph = RepoGraphBuilder::new(dir.path()).build().unwrap();
        let callees = callees_for_symbol(&graph, "beta", true);
        assert!(callees
            .iter()
            .any(|row| row.get("callee") == Some(&"alpha".to_string())));

        let docs = doc_links_for_symbol(&graph, "alpha", true);
        assert!(docs.iter().any(|row| row
            .get("doc_path")
            .map(|path| path.ends_with("docs/alpha.md"))
            .unwrap_or(false)));

        let dep_path = dep_path_between_symbols(&graph, "gamma", "alpha", true);
        assert!(dep_path.found);
        assert_eq!(dep_path.steps.len(), 2);

        let usage = usage_graph_for_symbol(&graph, "alpha", true, 2);
        assert!(usage.nodes.iter().any(|node| node.name == "beta"));

        let impact = impact_radius_for_symbol(&graph, "alpha", true, 2);
        assert!(impact.nodes.iter().any(|node| node.name == "gamma"));

        let context = context_bundle_for_query(&graph, "alpha behavior", 4, 2);
        assert!(context.text.contains("[symbol] alpha"));
        assert!(context
            .files
            .iter()
            .any(|path| path.ends_with("src/lib.rs")));
    }

    #[test]
    fn graph_status_marks_missing_files_as_stale() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn hello() {}\n").unwrap();

        let graph_path = default_graph_path_for_root(dir.path());
        build_and_save_graph(dir.path(), Some(&graph_path)).unwrap();
        fs::remove_file(dir.path().join("src/lib.rs")).unwrap();

        let status = graph_status(&graph_path);
        assert!(status.stale);
        assert_eq!(status.missing_files, 1);
    }

    #[test]
    fn refresh_graph_for_paths_rebuilds_only_changed_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        let graph_path = default_graph_path_for_root(dir.path());
        build_and_save_graph(dir.path(), Some(&graph_path)).unwrap();

        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha_renamed() {}\npub fn beta() { alpha_renamed(); }\n",
        )
        .unwrap();

        let refreshed = refresh_graph_for_paths(
            dir.path(),
            Some(&graph_path),
            &[PathBuf::from("src/lib.rs")],
        )
        .unwrap();
        assert_eq!(symbol_lookup(&refreshed, "alpha", true).len(), 0);
        assert_eq!(symbol_lookup(&refreshed, "alpha_renamed", true).len(), 1);
        let callees = callees_for_symbol(&refreshed, "beta", true);
        assert!(callees
            .iter()
            .any(|row| row.get("callee") == Some(&"alpha_renamed".to_string())));
    }

    #[test]
    fn indexes_extern_c_functions() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            r#"
use std::ffi::c_char;

pub extern "C" fn tubeai_version() -> *const c_char {
    std::ptr::null()
}
"#,
        )
        .unwrap();
        let graph = RepoGraphBuilder::new(dir.path()).build().unwrap();
        let matches = symbol_lookup(&graph, "tubeai_version", true);
        assert_eq!(matches.len(), 1);
    }
}
