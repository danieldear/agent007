use std::path::{Path, PathBuf};

use agent007_core::{
    build_and_save_graph, default_graph_path_for_root, load_graph, resolve_graph_path, RepoGraph,
};
use anyhow::{Context, Result};
use serde_json::Value;

pub fn input_root(input: &Value) -> Result<PathBuf> {
    let root = input["root"].as_str().unwrap_or(".");
    let path = PathBuf::from(root);
    Ok(path.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }))
}

pub fn input_graph_path(input: &Value, root: &Path) -> PathBuf {
    input["graph_path"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_graph_path_for_root(root))
}

pub fn load_existing_graph(input: &Value) -> Result<(PathBuf, RepoGraph)> {
    let root = input_root(input)?;
    let graph_path = input_graph_path(input, &root);
    let graph = load_graph(&graph_path)
        .with_context(|| format!("failed to load {}", graph_path.display()))?;
    Ok((graph_path, graph))
}

pub fn load_graph_maybe_build(input: &Value) -> Result<(PathBuf, RepoGraph)> {
    let root = input_root(input)?;
    let graph_path = input_graph_path(input, &root);
    let build_if_missing = input["build_if_missing"].as_bool().unwrap_or(false);
    if graph_path.exists() {
        let graph = load_graph(&graph_path)
            .with_context(|| format!("failed to load {}", graph_path.display()))?;
        return Ok((graph_path, graph));
    }
    if build_if_missing {
        let graph = build_and_save_graph(&root, Some(&graph_path))
            .with_context(|| format!("failed to build {}", graph_path.display()))?;
        return Ok((graph_path, graph));
    }
    anyhow::bail!(
        "graph not found at {}; run etr.graph_build first or pass build_if_missing=true",
        graph_path.display()
    )
}

#[allow(dead_code)]
pub fn resolved_graph_path(input: &Value) -> PathBuf {
    let root = input["root"].as_str().map(PathBuf::from);
    let explicit = input["graph_path"].as_str().map(PathBuf::from);
    resolve_graph_path(root.as_deref(), explicit.as_deref())
}
