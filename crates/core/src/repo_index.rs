use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use redb::{
    Database, MultimapTable, MultimapTableDefinition, ReadableDatabase, ReadableMultimapTable,
    ReadableTable, Table, TableDefinition,
};
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::hash::stable_context_hash;
use crate::repo_graph::{
    RepoGraph, RepoGraphCounts, RepoGraphEdge, RepoGraphEdgeKind, RepoGraphNeighborhood,
    RepoGraphNode, RepoGraphNodeKind, RepoGraphQueryContext,
};

const INDEX_VERSION: u32 = 2;
const INDEX_FILENAME: &str = "repo_index_v2.redb";

const META: TableDefinition<&str, &str> = TableDefinition::new("meta");
const NODES: TableDefinition<&str, &[u8]> = TableDefinition::new("nodes");
const EDGES: TableDefinition<&str, &[u8]> = TableDefinition::new("edges");
const SYMBOL_NAME: MultimapTableDefinition<&str, &str> =
    MultimapTableDefinition::new("symbol_name");
const MODULE_NAME: MultimapTableDefinition<&str, &str> =
    MultimapTableDefinition::new("module_name");
const EDGES_FROM: MultimapTableDefinition<&str, &str> = MultimapTableDefinition::new("edges_from");
const EDGES_TO: MultimapTableDefinition<&str, &str> = MultimapTableDefinition::new("edges_to");
const FILE_NODES: MultimapTableDefinition<&str, &str> = MultimapTableDefinition::new("file_nodes");
const FILE_EDGES: MultimapTableDefinition<&str, &str> = MultimapTableDefinition::new("file_edges");

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoIndexStatus {
    pub exists: bool,
    pub index_path: String,
    pub root: Option<String>,
    pub built_at: Option<String>,
    pub version: Option<u32>,
    pub counts: Option<RepoGraphCounts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn default_index_path_for_root(root: &Path) -> PathBuf {
    root.join(".agent007").join("runtime").join(INDEX_FILENAME)
}

pub fn index_path_for_graph_path(graph_path: &Path) -> PathBuf {
    graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(INDEX_FILENAME)
}

pub fn index_status(path: &Path) -> RepoIndexStatus {
    if !path.exists() {
        return RepoIndexStatus {
            exists: false,
            index_path: path.display().to_string(),
            root: None,
            built_at: None,
            version: None,
            counts: None,
            error: None,
        };
    }
    match RepoIndex::open(path).and_then(|index| index.status()) {
        Ok(mut status) => {
            status.exists = true;
            status.index_path = path.display().to_string();
            status
        }
        Err(error) => RepoIndexStatus {
            exists: true,
            index_path: path.display().to_string(),
            root: None,
            built_at: None,
            version: None,
            counts: None,
            error: Some(error.to_string()),
        },
    }
}

pub struct RepoIndexSink<'txn> {
    nodes: Table<'txn, &'static str, &'static [u8]>,
    edges: Table<'txn, &'static str, &'static [u8]>,
    symbol_name: MultimapTable<'txn, &'static str, &'static str>,
    module_name: MultimapTable<'txn, &'static str, &'static str>,
    edges_from: MultimapTable<'txn, &'static str, &'static str>,
    edges_to: MultimapTable<'txn, &'static str, &'static str>,
    file_nodes: MultimapTable<'txn, &'static str, &'static str>,
    file_edges: MultimapTable<'txn, &'static str, &'static str>,
}

impl RepoIndexSink<'_> {
    pub fn insert_node(&mut self, node: &RepoGraphNode) -> Result<bool, CoreError> {
        let raw = serde_json::to_vec(node)?;
        let inserted = self
            .nodes
            .insert(node.id.as_str(), raw.as_slice())
            .map_err(|e| CoreError::repo_index(format!("write repo index node: {e}")))?
            .is_none();
        if !inserted {
            return Ok(false);
        }
        if node.kind == RepoGraphNodeKind::Symbol {
            self.symbol_name
                .insert(node.name.to_lowercase().as_str(), node.id.as_str())
                .map_err(|e| CoreError::repo_index(format!("write repo index symbol_name: {e}")))?;
        } else if node.kind == RepoGraphNodeKind::Module {
            self.module_name
                .insert(node.name.to_lowercase().as_str(), node.id.as_str())
                .map_err(|e| CoreError::repo_index(format!("write repo index module_name: {e}")))?;
        }
        if let Some(path) = &node.path {
            self.file_nodes
                .insert(path.as_str(), node.id.as_str())
                .map_err(|e| CoreError::repo_index(format!("write repo index file_nodes: {e}")))?;
        }
        Ok(inserted)
    }

    pub fn insert_edge(&mut self, edge: &RepoGraphEdge) -> Result<bool, CoreError> {
        let edge_id = stable_edge_id(edge);
        let raw = serde_json::to_vec(edge)?;
        let inserted = self
            .edges
            .insert(edge_id.as_str(), raw.as_slice())
            .map_err(|e| CoreError::repo_index(format!("write repo index edge: {e}")))?
            .is_none();
        if !inserted {
            return Ok(false);
        }
        self.edges_from
            .insert(edge.from.as_str(), edge_id.as_str())
            .map_err(|e| CoreError::repo_index(format!("write repo index edges_from: {e}")))?;
        self.edges_to
            .insert(edge.to.as_str(), edge_id.as_str())
            .map_err(|e| CoreError::repo_index(format!("write repo index edges_to: {e}")))?;
        if let Some(path) = &edge.path {
            self.file_edges
                .insert(path.as_str(), edge_id.as_str())
                .map_err(|e| CoreError::repo_index(format!("write repo index file_edges: {e}")))?;
        }
        Ok(inserted)
    }
}

pub fn write_index_with(
    path: &Path,
    root: &str,
    built_at: &str,
    mut write_fn: impl FnMut(&mut RepoIndexSink<'_>) -> Result<RepoGraphCounts, CoreError>,
) -> Result<RepoIndexStatus, CoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
    }
    if path.exists() {
        fs::remove_file(path).map_err(|e| CoreError::io(path, e))?;
    }
    let db = Database::create(path)
        .map_err(|e| CoreError::repo_index(format!("create repo index: {e}")))?;
    let write = db
        .begin_write()
        .map_err(|e| CoreError::repo_index(format!("open repo index writer: {e}")))?;

    let counts = {
        let mut sink = RepoIndexSink {
            nodes: write
                .open_table(NODES)
                .map_err(|e| CoreError::repo_index(format!("open repo index nodes: {e}")))?,
            edges: write
                .open_table(EDGES)
                .map_err(|e| CoreError::repo_index(format!("open repo index edges: {e}")))?,
            symbol_name: write
                .open_multimap_table(SYMBOL_NAME)
                .map_err(|e| CoreError::repo_index(format!("open repo index symbol_name: {e}")))?,
            module_name: write
                .open_multimap_table(MODULE_NAME)
                .map_err(|e| CoreError::repo_index(format!("open repo index module_name: {e}")))?,
            edges_from: write
                .open_multimap_table(EDGES_FROM)
                .map_err(|e| CoreError::repo_index(format!("open repo index edges_from: {e}")))?,
            edges_to: write
                .open_multimap_table(EDGES_TO)
                .map_err(|e| CoreError::repo_index(format!("open repo index edges_to: {e}")))?,
            file_nodes: write
                .open_multimap_table(FILE_NODES)
                .map_err(|e| CoreError::repo_index(format!("open repo index file_nodes: {e}")))?,
            file_edges: write
                .open_multimap_table(FILE_EDGES)
                .map_err(|e| CoreError::repo_index(format!("open repo index file_edges: {e}")))?,
        };
        write_fn(&mut sink)?
    };

    {
        let mut meta = write
            .open_table(META)
            .map_err(|e| CoreError::repo_index(format!("open repo index meta: {e}")))?;
        meta.insert("format", "repo_index")
            .map_err(|e| CoreError::repo_index(format!("write repo index meta: {e}")))?;
        meta.insert("version", INDEX_VERSION.to_string().as_str())
            .map_err(|e| CoreError::repo_index(format!("write repo index version: {e}")))?;
        meta.insert("root", root)
            .map_err(|e| CoreError::repo_index(format!("write repo index root: {e}")))?;
        meta.insert("built_at", built_at)
            .map_err(|e| CoreError::repo_index(format!("write repo index built_at: {e}")))?;
        let counts_json = serde_json::to_string(&counts)?;
        meta.insert("counts", counts_json.as_str())
            .map_err(|e| CoreError::repo_index(format!("write repo index counts: {e}")))?;
    }

    write
        .commit()
        .map_err(|e| CoreError::repo_index(format!("commit repo index: {e}")))?;

    Ok(RepoIndexStatus {
        exists: true,
        index_path: path.display().to_string(),
        root: Some(root.to_string()),
        built_at: Some(built_at.to_string()),
        version: Some(INDEX_VERSION),
        counts: Some(counts),
        error: None,
    })
}

fn stable_edge_id(edge: &RepoGraphEdge) -> String {
    let key = format!(
        "{:?}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
        edge.kind,
        edge.from,
        edge.to,
        edge.path.clone().unwrap_or_default(),
        edge.line.unwrap_or(0)
    );
    format!("edge:{:016x}", stable_context_hash(&key))
}

pub fn save_index(graph: &RepoGraph, path: &Path) -> Result<(), CoreError> {
    write_index_with(path, &graph.root, &graph.built_at, |sink| {
        for node in &graph.nodes {
            let _ = sink.insert_node(node)?;
        }
        for edge in &graph.edges {
            let _ = sink.insert_edge(edge)?;
        }
        Ok(graph.counts.clone())
    })?;
    Ok(())
}

pub fn build_and_save_index_for_graph(graph: &RepoGraph) -> Result<PathBuf, CoreError> {
    let path = index_path_for_graph_path(Path::new(&graph.graph_path));
    save_index(graph, &path)?;
    Ok(path)
}

pub struct RepoIndex {
    path: PathBuf,
    db: Database,
}

impl RepoIndex {
    pub fn open(path: &Path) -> Result<Self, CoreError> {
        let db = Database::open(path).map_err(|e| {
            CoreError::repo_index(format!("open repo index {}: {e}", path.display()))
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            db,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn status(&self) -> Result<RepoIndexStatus, CoreError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| CoreError::repo_index(format!("read repo index status: {e}")))?;
        let meta = read
            .open_table(META)
            .map_err(|e| CoreError::repo_index(format!("open repo index meta: {e}")))?;
        let root = get_meta_string(&meta, "root")?;
        let built_at = get_meta_string(&meta, "built_at")?;
        let version = get_meta_string(&meta, "version")?.and_then(|v| v.parse().ok());
        let counts = get_meta_string(&meta, "counts")?
            .and_then(|v| serde_json::from_str::<RepoGraphCounts>(&v).ok());
        Ok(RepoIndexStatus {
            exists: true,
            index_path: self.path.display().to_string(),
            root,
            built_at,
            version,
            counts,
            error: None,
        })
    }

    pub fn symbol_lookup(
        &self,
        symbol: &str,
        exact: bool,
    ) -> Result<Vec<RepoGraphNode>, CoreError> {
        let needle = symbol.to_lowercase();
        let read = self
            .db
            .begin_read()
            .map_err(|e| CoreError::repo_index(format!("read repo index: {e}")))?;
        let nodes = read
            .open_table(NODES)
            .map_err(|e| CoreError::repo_index(format!("open repo index nodes: {e}")))?;
        let symbol_name = read
            .open_multimap_table(SYMBOL_NAME)
            .map_err(|e| CoreError::repo_index(format!("open repo index symbol_name: {e}")))?;
        let module_name = read
            .open_multimap_table(MODULE_NAME)
            .map_err(|e| CoreError::repo_index(format!("open repo index module_name: {e}")))?;
        let mut ids = BTreeSet::new();
        if exact {
            collect_multimap_values(&symbol_name, needle.as_str(), &mut ids)?;
            collect_multimap_values(&module_name, needle.as_str(), &mut ids)?;
        } else {
            collect_matching_multimap_values(&symbol_name, &needle, &mut ids)?;
            collect_matching_multimap_values(&module_name, &needle, &mut ids)?;
        }
        let mut out = Vec::new();
        for id in ids {
            if let Some(node) = get_node_from_table(&nodes, &id)? {
                out.push(node);
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
        Ok(out)
    }

    pub fn callers_for_symbol(
        &self,
        symbol: &str,
        exact: bool,
    ) -> Result<Vec<BTreeMap<String, String>>, CoreError> {
        let targets = self.symbol_lookup(symbol, exact)?;
        let target_ids: BTreeSet<String> = targets.iter().map(|node| node.id.clone()).collect();
        if target_ids.is_empty() {
            return Ok(Vec::new());
        }
        let read = self
            .db
            .begin_read()
            .map_err(|e| CoreError::repo_index(format!("read repo index: {e}")))?;
        let nodes = read
            .open_table(NODES)
            .map_err(|e| CoreError::repo_index(format!("open repo index nodes: {e}")))?;
        let edges = read
            .open_table(EDGES)
            .map_err(|e| CoreError::repo_index(format!("open repo index edges: {e}")))?;
        let edges_to = read
            .open_multimap_table(EDGES_TO)
            .map_err(|e| CoreError::repo_index(format!("open repo index edges_to: {e}")))?;
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        for target in target_ids {
            for edge_id in multimap_values(&edges_to, target.as_str())? {
                let Some(edge) = get_edge_from_table(&edges, &edge_id)? else {
                    continue;
                };
                if edge.kind != RepoGraphEdgeKind::Calls {
                    continue;
                }
                let Some(from_node) = get_node_from_table(&nodes, &edge.from)? else {
                    continue;
                };
                let Some(to_node) = get_node_from_table(&nodes, &edge.to)? else {
                    continue;
                };
                let key = unique_edge_key(&from_node, &to_node, &edge);
                if seen.insert(key) {
                    rows.push(call_row("caller", "callee", &from_node, &to_node, &edge));
                }
            }
        }
        Ok(rows)
    }

    pub fn callees_for_symbol(
        &self,
        symbol: &str,
        exact: bool,
    ) -> Result<Vec<BTreeMap<String, String>>, CoreError> {
        let targets = self.symbol_lookup(symbol, exact)?;
        let target_ids: BTreeSet<String> = targets.iter().map(|node| node.id.clone()).collect();
        if target_ids.is_empty() {
            return Ok(Vec::new());
        }
        let read = self
            .db
            .begin_read()
            .map_err(|e| CoreError::repo_index(format!("read repo index: {e}")))?;
        let nodes = read
            .open_table(NODES)
            .map_err(|e| CoreError::repo_index(format!("open repo index nodes: {e}")))?;
        let edges = read
            .open_table(EDGES)
            .map_err(|e| CoreError::repo_index(format!("open repo index edges: {e}")))?;
        let edges_from = read
            .open_multimap_table(EDGES_FROM)
            .map_err(|e| CoreError::repo_index(format!("open repo index edges_from: {e}")))?;
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        for target in target_ids {
            for edge_id in multimap_values(&edges_from, target.as_str())? {
                let Some(edge) = get_edge_from_table(&edges, &edge_id)? else {
                    continue;
                };
                if edge.kind != RepoGraphEdgeKind::Calls {
                    continue;
                }
                let Some(from_node) = get_node_from_table(&nodes, &edge.from)? else {
                    continue;
                };
                let Some(to_node) = get_node_from_table(&nodes, &edge.to)? else {
                    continue;
                };
                let key = unique_edge_key(&from_node, &to_node, &edge);
                if seen.insert(key) {
                    rows.push(call_row("caller", "callee", &from_node, &to_node, &edge));
                }
            }
        }
        Ok(rows)
    }

    pub fn doc_links_for_symbol(
        &self,
        symbol: &str,
        exact: bool,
    ) -> Result<Vec<BTreeMap<String, String>>, CoreError> {
        let targets = self.symbol_lookup(symbol, exact)?;
        let target_ids: BTreeSet<String> = targets.iter().map(|node| node.id.clone()).collect();
        if target_ids.is_empty() {
            return Ok(Vec::new());
        }
        let read = self
            .db
            .begin_read()
            .map_err(|e| CoreError::repo_index(format!("read repo index: {e}")))?;
        let nodes = read
            .open_table(NODES)
            .map_err(|e| CoreError::repo_index(format!("open repo index nodes: {e}")))?;
        let edges = read
            .open_table(EDGES)
            .map_err(|e| CoreError::repo_index(format!("open repo index edges: {e}")))?;
        let edges_to = read
            .open_multimap_table(EDGES_TO)
            .map_err(|e| CoreError::repo_index(format!("open repo index edges_to: {e}")))?;
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        for target in target_ids {
            for edge_id in multimap_values(&edges_to, target.as_str())? {
                let Some(edge) = get_edge_from_table(&edges, &edge_id)? else {
                    continue;
                };
                if edge.kind != RepoGraphEdgeKind::Documents {
                    continue;
                }
                let Some(doc_node) = get_node_from_table(&nodes, &edge.from)? else {
                    continue;
                };
                let Some(symbol_node) = get_node_from_table(&nodes, &edge.to)? else {
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
        }
        Ok(rows)
    }

    pub fn context_bundle_for_query(
        &self,
        query: &str,
        max_symbols: usize,
        max_neighbors: usize,
    ) -> Result<RepoGraphQueryContext, CoreError> {
        let keywords = extract_query_keywords(query);
        let mut matched_symbols = Vec::new();
        let mut seen_ids = BTreeSet::new();
        for keyword in keywords {
            let mut matches = self.symbol_lookup(&keyword, true)?;
            if matches.is_empty() {
                matches = self.symbol_lookup(&keyword, false)?;
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

        let read = self
            .db
            .begin_read()
            .map_err(|e| CoreError::repo_index(format!("read repo index: {e}")))?;
        let nodes = read
            .open_table(NODES)
            .map_err(|e| CoreError::repo_index(format!("open repo index nodes: {e}")))?;
        let mut files = BTreeSet::new();
        let mut related_docs = Vec::new();
        let mut text = String::new();
        let mut seen_docs = BTreeSet::new();
        for node in matched_symbols.iter().take(max_symbols) {
            if let Some(path) = &node.path {
                files.insert(path.clone());
            }
            text.push_str(&format!(
                "[symbol] {} ({})\n",
                node.name,
                node.path.clone().unwrap_or_default()
            ));

            for row in self
                .callers_for_symbol(&node.name, true)?
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

            for row in self
                .callees_for_symbol(&node.name, true)?
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

            for row in self
                .doc_links_for_symbol(&node.name, true)?
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
                    let doc_id = format!("doc:{doc_path}");
                    if seen_docs.insert(doc_id.clone()) {
                        if let Some(doc_node) = get_node_from_table(&nodes, &doc_id)? {
                            related_docs.push(doc_node);
                        }
                    }
                }
            }
            text.push('\n');
        }

        Ok(RepoGraphQueryContext {
            query: query.to_string(),
            matched_symbols,
            related_docs,
            files: files.into_iter().collect(),
            text: text.trim().to_string(),
        })
    }

    pub fn usage_graph_for_symbol(
        &self,
        symbol: &str,
        exact: bool,
        max_depth: usize,
    ) -> Result<RepoGraphNeighborhood, CoreError> {
        let matched_symbols = self.symbol_lookup(symbol, exact)?;
        let seed_ids: Vec<String> = matched_symbols.iter().map(|node| node.id.clone()).collect();
        let depth = max_depth.max(1);
        let read = self
            .db
            .begin_read()
            .map_err(|e| CoreError::repo_index(format!("read repo index: {e}")))?;
        let nodes_table = read
            .open_table(NODES)
            .map_err(|e| CoreError::repo_index(format!("open repo index nodes: {e}")))?;
        let edges_table = read
            .open_table(EDGES)
            .map_err(|e| CoreError::repo_index(format!("open repo index edges: {e}")))?;
        let edges_from = read
            .open_multimap_table(EDGES_FROM)
            .map_err(|e| CoreError::repo_index(format!("open repo index edges_from: {e}")))?;
        let edges_to = read
            .open_multimap_table(EDGES_TO)
            .map_err(|e| CoreError::repo_index(format!("open repo index edges_to: {e}")))?;

        let mut seen_nodes = BTreeSet::new();
        let mut seen_edges = BTreeSet::new();
        let mut queue = VecDeque::new();
        for id in &seed_ids {
            seen_nodes.insert(id.clone());
            queue.push_back((id.clone(), 0usize));
        }
        while let Some((node_id, current_depth)) = queue.pop_front() {
            if current_depth >= depth {
                continue;
            }
            for edge_id in multimap_values(&edges_from, node_id.as_str())?
                .into_iter()
                .chain(multimap_values(&edges_to, node_id.as_str())?)
            {
                if !seen_edges.insert(edge_id.clone()) {
                    continue;
                }
                let Some(edge) = get_edge_from_table(&edges_table, &edge_id)? else {
                    continue;
                };
                for next in [&edge.from, &edge.to] {
                    if seen_nodes.insert(next.clone()) {
                        queue.push_back((next.clone(), current_depth + 1));
                    }
                }
            }
        }
        let mut nodes = Vec::new();
        for id in &seen_nodes {
            if let Some(node) = get_node_from_table(&nodes_table, id)? {
                nodes.push(node);
            }
        }
        let mut edges = Vec::new();
        for id in &seen_edges {
            if let Some(edge) = get_edge_from_table(&edges_table, id)? {
                if seen_nodes.contains(&edge.from) && seen_nodes.contains(&edge.to) {
                    edges.push(edge);
                }
            }
        }
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        edges.sort_by(|a, b| a.from.cmp(&b.from).then(a.to.cmp(&b.to)));
        Ok(RepoGraphNeighborhood {
            symbol: symbol.to_string(),
            exact,
            max_depth: depth,
            matched_symbols,
            nodes,
            edges,
        })
    }
}

fn get_meta_string(
    table: &impl ReadableTable<&'static str, &'static str>,
    key: &str,
) -> Result<Option<String>, CoreError> {
    table
        .get(key)
        .map_err(|e| CoreError::repo_index(format!("read repo index meta: {e}")))
        .map(|value| value.map(|v| v.value().to_string()))
}

fn multimap_values(
    table: &impl ReadableMultimapTable<&'static str, &'static str>,
    key: &str,
) -> Result<Vec<String>, CoreError> {
    let values = table
        .get(key)
        .map_err(|e| CoreError::repo_index(format!("read repo index multimap: {e}")))?;
    values
        .map(|value| {
            value
                .map(|v| v.value().to_string())
                .map_err(|e| CoreError::repo_index(format!("read repo index multimap value: {e}")))
        })
        .collect()
}

fn collect_multimap_values(
    table: &impl ReadableMultimapTable<&'static str, &'static str>,
    key: &str,
    out: &mut BTreeSet<String>,
) -> Result<(), CoreError> {
    for value in multimap_values(table, key)? {
        out.insert(value);
    }
    Ok(())
}

fn collect_matching_multimap_values(
    table: &impl ReadableMultimapTable<&'static str, &'static str>,
    needle: &str,
    out: &mut BTreeSet<String>,
) -> Result<(), CoreError> {
    let range = table
        .iter()
        .map_err(|e| CoreError::repo_index(format!("scan repo index multimap: {e}")))?;
    for entry in range {
        let (key, values) =
            entry.map_err(|e| CoreError::repo_index(format!("read repo index key: {e}")))?;
        if key.value().contains(needle) {
            for value in values {
                out.insert(
                    value.map(|v| v.value().to_string()).map_err(|e| {
                        CoreError::repo_index(format!("read repo index value: {e}"))
                    })?,
                );
            }
        }
    }
    Ok(())
}

fn get_node_from_table(
    table: &impl ReadableTable<&'static str, &'static [u8]>,
    id: &str,
) -> Result<Option<RepoGraphNode>, CoreError> {
    let Some(raw) = table
        .get(id)
        .map_err(|e| CoreError::repo_index(format!("read repo index node: {e}")))?
    else {
        return Ok(None);
    };
    serde_json::from_slice(raw.value())
        .map(Some)
        .map_err(CoreError::from)
}

fn get_edge_from_table(
    table: &impl ReadableTable<&'static str, &'static [u8]>,
    id: &str,
) -> Result<Option<RepoGraphEdge>, CoreError> {
    let Some(raw) = table
        .get(id)
        .map_err(|e| CoreError::repo_index(format!("read repo index edge: {e}")))?
    else {
        return Ok(None);
    };
    serde_json::from_slice(raw.value())
        .map(Some)
        .map_err(CoreError::from)
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
    row.insert(
        format!("{from_label}_line"),
        from_node
            .line
            .map(|line| line.to_string())
            .unwrap_or_default(),
    );
    row.insert(to_label.into(), to_node.name.clone());
    row.insert(
        format!("{to_label}_path"),
        to_node.path.clone().unwrap_or_default(),
    );
    row.insert(
        format!("{to_label}_line"),
        to_node
            .line
            .map(|line| line.to_string())
            .unwrap_or_default(),
    );
    row.insert("edge_path".into(), edge.path.clone().unwrap_or_default());
    row.insert(
        "edge_line".into(),
        edge.line.map(|line| line.to_string()).unwrap_or_default(),
    );
    row
}

fn unique_edge_key(
    from_node: &RepoGraphNode,
    to_node: &RepoGraphNode,
    edge: &RepoGraphEdge,
) -> (String, String, Option<String>, Option<usize>) {
    (
        from_node.id.clone(),
        to_node.id.clone(),
        edge.path.clone(),
        edge.line,
    )
}

pub fn load_or_build_index(root: &Path, graph_path: Option<&Path>) -> Result<RepoIndex, CoreError> {
    let graph_path = crate::repo_graph::resolve_graph_path(Some(root), graph_path);
    let index_path = index_path_for_graph_path(&graph_path);
    if index_path.exists() {
        return RepoIndex::open(&index_path);
    }
    crate::repo_graph::build_and_save_index(root, Some(&index_path))?;
    RepoIndex::open(&index_path)
}

pub fn context_bundle_for_query_index(
    root: &Path,
    graph_path: Option<&Path>,
    query: &str,
    max_symbols: usize,
    max_neighbors: usize,
) -> Result<RepoGraphQueryContext, CoreError> {
    let index = load_or_build_index(root, graph_path)?;
    index.context_bundle_for_query(query, max_symbols, max_neighbors)
}

pub fn evidence_refs_for_text_index(
    root: &Path,
    graph_path: Option<&Path>,
    text: &str,
    limit: usize,
) -> Vec<String> {
    let Ok(bundle) = context_bundle_for_query_index(root, graph_path, text, limit.max(1), 1) else {
        return Vec::new();
    };
    bundle
        .matched_symbols
        .into_iter()
        .take(limit)
        .map(|node| node.id)
        .collect()
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

pub fn open_index_for_graph_path(graph_path: &Path) -> Result<Option<RepoIndex>, CoreError> {
    let path = index_path_for_graph_path(graph_path);
    if !path.exists() {
        return Ok(None);
    }
    RepoIndex::open(&path).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_graph::RepoGraphBuilder;

    #[test]
    fn index_answers_symbol_and_call_queries_without_json_load() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        let graph = RepoGraphBuilder::new(dir.path()).build().unwrap();
        let index_path = default_index_path_for_root(dir.path());
        save_index(&graph, &index_path).unwrap();
        let index = RepoIndex::open(&index_path).unwrap();
        assert_eq!(index.symbol_lookup("alpha", true).unwrap().len(), 1);
        assert!(!index.callers_for_symbol("alpha", true).unwrap().is_empty());
        assert!(!index.callees_for_symbol("beta", true).unwrap().is_empty());
        assert!(!index
            .usage_graph_for_symbol("alpha", true, 1)
            .unwrap()
            .nodes
            .is_empty());
    }

    #[test]
    fn sink_deduplicates_secondary_indexes_for_repeated_nodes_and_edges() {
        let dir = tempfile::tempdir().unwrap();
        let index_path = default_index_path_for_root(dir.path());
        let node = RepoGraphNode {
            id: "symbol:src/lib.rs:alpha:1".into(),
            kind: RepoGraphNodeKind::Symbol,
            name: "Alpha".into(),
            path: Some("src/lib.rs".into()),
            language: Some("rust".into()),
            symbol_kind: Some("function".into()),
            line: Some(1),
            signature: Some("fn alpha()".into()),
        };
        let edge = RepoGraphEdge {
            kind: RepoGraphEdgeKind::Defines,
            from: "file:src/lib.rs".into(),
            to: node.id.clone(),
            path: Some("src/lib.rs".into()),
            line: Some(1),
        };
        let edge_id = stable_edge_id(&edge);

        write_index_with(
            &index_path,
            dir.path().to_str().unwrap(),
            "2026-06-30T00:00:00Z",
            |sink| {
                assert!(sink.insert_node(&node)?);
                assert!(!sink.insert_node(&node)?);
                assert!(sink.insert_edge(&edge)?);
                assert!(!sink.insert_edge(&edge)?);
                Ok(RepoGraphCounts {
                    symbols: 1,
                    edges: 1,
                    ..RepoGraphCounts::default()
                })
            },
        )
        .unwrap();

        let index = RepoIndex::open(&index_path).unwrap();
        let read = index.db.begin_read().unwrap();
        let symbol_name = read.open_multimap_table(SYMBOL_NAME).unwrap();
        let file_nodes = read.open_multimap_table(FILE_NODES).unwrap();
        let edges_from = read.open_multimap_table(EDGES_FROM).unwrap();
        let edges_to = read.open_multimap_table(EDGES_TO).unwrap();
        let file_edges = read.open_multimap_table(FILE_EDGES).unwrap();

        assert_eq!(
            multimap_values(&symbol_name, "alpha").unwrap(),
            vec![node.id.clone()]
        );
        assert_eq!(
            multimap_values(&file_nodes, "src/lib.rs").unwrap(),
            vec![node.id.clone()]
        );
        assert_eq!(
            multimap_values(&edges_from, "file:src/lib.rs").unwrap(),
            vec![edge_id.clone()]
        );
        assert_eq!(
            multimap_values(&edges_to, node.id.as_str()).unwrap(),
            vec![edge_id.clone()]
        );
        assert_eq!(
            multimap_values(&file_edges, "src/lib.rs").unwrap(),
            vec![edge_id]
        );
    }

    #[test]
    fn index_status_reports_corrupt_index_errors() {
        let dir = tempfile::tempdir().unwrap();
        let index_path = default_index_path_for_root(dir.path());
        std::fs::create_dir_all(index_path.parent().unwrap()).unwrap();
        std::fs::write(&index_path, b"not a redb database").unwrap();

        let status = index_status(&index_path);

        assert!(status.exists);
        assert_eq!(status.index_path, index_path.display().to_string());
        assert!(status.root.is_none());
        assert!(status.counts.is_none());
        assert!(status.error.unwrap_or_default().contains("repo index"));
    }
}
