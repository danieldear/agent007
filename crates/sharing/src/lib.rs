use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A single skill or workflow packed into a bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleAsset {
    /// Relative path inside the target skills/ or workflows/ directory.
    /// For backward compatibility this field keeps the historical name `filename`.
    pub filename: String,
    pub content: String,
    pub sha256: String,
}

impl BundleAsset {
    pub fn new(filename: impl Into<String>, content: impl Into<String>) -> Self {
        let content = content.into();
        let sha256 = hex::encode(Sha256::digest(content.as_bytes()));
        Self {
            filename: filename.into(),
            content,
            sha256,
        }
    }

    pub fn verify(&self) -> bool {
        hex::encode(Sha256::digest(self.content.as_bytes())) == self.sha256
    }
}

/// A portable bundle of skills and/or workflows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub version: String,
    pub created_at: String,
    pub skills: Vec<BundleAsset>,
    pub workflows: Vec<BundleAsset>,
    #[serde(default)]
    pub tools: Vec<BundleAsset>,
    #[serde(default)]
    pub personas: Vec<BundleAsset>,
}

impl Bundle {
    pub fn new(
        skills: Vec<BundleAsset>,
        workflows: Vec<BundleAsset>,
        tools: Vec<BundleAsset>,
        personas: Vec<BundleAsset>,
    ) -> Self {
        Self {
            version: "1".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            skills,
            workflows,
            tools,
            personas,
        }
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("serialize bundle")
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).context("parse bundle")
    }
}

/// Builds a bundle from files on disk.
/// Accepts multiple skills, workflows, and persona directories so that project-local
/// and global agent007 homes can both be searched (matching the listing APIs).
pub struct BundleBuilder {
    skills_dirs: Vec<PathBuf>,
    workflows_dirs: Vec<PathBuf>,
    tools_dirs: Vec<PathBuf>,
    scripts_dirs: Vec<PathBuf>,
    persona_dirs: Vec<PathBuf>,
}

impl BundleBuilder {
    pub fn new(
        skills_dirs: impl IntoIterator<Item = impl Into<PathBuf>>,
        workflows_dirs: impl IntoIterator<Item = impl Into<PathBuf>>,
    ) -> Self {
        let skills_dirs: Vec<PathBuf> = skills_dirs.into_iter().map(|p| p.into()).collect();
        let workflows_dirs: Vec<PathBuf> = workflows_dirs.into_iter().map(|p| p.into()).collect();
        // Derive tools dirs from the parent of each skills dir, deduplicated.
        let mut seen_tools: HashSet<PathBuf> = HashSet::new();
        let tools_dirs: Vec<PathBuf> = skills_dirs
            .iter()
            .filter_map(|p| p.parent().map(|parent| parent.join("tools")))
            .filter(|p| seen_tools.insert(p.clone()))
            .collect();
        // Derive scripts dirs from the parent of each skills dir, deduplicated.
        let mut seen_scripts: HashSet<PathBuf> = HashSet::new();
        let scripts_dirs: Vec<PathBuf> = skills_dirs
            .iter()
            .filter_map(|p| p.parent().map(|parent| parent.join("scripts")))
            .filter(|p| seen_scripts.insert(p.clone()))
            .collect();
        // Derive persona dirs from the parent of each skills dir (same home), deduplicated.
        let mut seen_personas: HashSet<PathBuf> = HashSet::new();
        let persona_dirs: Vec<PathBuf> = skills_dirs
            .iter()
            .filter_map(|p| p.parent().map(|parent| parent.join("personas")))
            .filter(|p| seen_personas.insert(p.clone()))
            .collect();
        Self {
            skills_dirs,
            workflows_dirs,
            tools_dirs,
            scripts_dirs,
            persona_dirs,
        }
    }

    /// Override the persona search directories (used in tests and when callers need fine control).
    pub fn with_persona_dirs(mut self, dirs: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        self.persona_dirs = dirs.into_iter().map(|p| p.into()).collect();
        self
    }

    /// Build a bundle containing the specified skills (by trigger), workflows (by name),
    /// personas (by name), and optionally a filtered set of tool paths.
    ///
    /// Personas are collected as the union of:
    ///   - explicitly requested names (persona_names non-empty → only those names)
    ///   - names auto-detected from the `agent:` fields of selected workflows/skills
    ///
    /// `tool_filter`: empty = auto-include all detected tools; `["__none__"]` = include none;
    /// otherwise only include tools whose bundle path is in the filter.
    pub fn build(
        &self,
        skill_triggers: &[&str],
        workflow_names: &[&str],
        persona_names: &[&str],
        tool_filter: &[&str],
    ) -> Result<Bundle> {
        let workflows = self.collect_workflow_assets(workflow_names)?;
        let skills = self.collect_skill_assets_with_workflow_deps(skill_triggers, &workflows)?;
        let tools = self.collect_tools_assets(&skills, &workflows, tool_filter)?;
        let personas = self.collect_persona_assets(&skills, &workflows, persona_names)?;
        Ok(Bundle::new(skills, workflows, tools, personas))
    }

    fn collect_skill_assets_with_workflow_deps(
        &self,
        requested_skills: &[&str],
        workflows: &[BundleAsset],
    ) -> Result<Vec<BundleAsset>> {
        let mut filter: HashSet<String> = normalize_skill_filter(requested_skills);
        for workflow in workflows {
            let mut refs = HashSet::new();
            collect_workflow_skill_refs(&workflow.content, &mut refs);
            for skill_ref in refs {
                if skill_ref.is_empty() {
                    continue;
                }
                filter.insert(skill_ref.clone());
                filter.insert(skill_ref.trim_start_matches('/').to_string());
            }
        }
        if filter.is_empty() {
            return self.collect_skill_assets(&[]);
        }
        let refs: Vec<&str> = filter.iter().map(String::as_str).collect();
        self.collect_skill_assets(&refs)
    }

    fn collect_skill_assets(&self, filter: &[&str]) -> Result<Vec<BundleAsset>> {
        let normalized_filter = normalize_skill_filter(filter);
        let mut assets: Vec<BundleAsset> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for skills_dir in &self.skills_dirs {
            if !skills_dir.exists() {
                continue;
            }
            for entry in std::fs::read_dir(skills_dir)?.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if path.extension().and_then(|e| e.to_str()) != Some("md") {
                        continue;
                    }
                    let content = std::fs::read_to_string(&path)?;
                    let stem = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();
                    let trigger = parse_frontmatter_value(&content, "trigger")
                        .map(|v| normalize_trigger_key(&v));
                    if !filter_matches_skill(&normalized_filter, &stem, trigger.as_deref()) {
                        continue;
                    }
                    let rel = path
                        .strip_prefix(skills_dir)
                        .ok()
                        .and_then(|p| p.to_str())
                        .unwrap_or_default()
                        .replace('\\', "/");
                    if !rel.is_empty() && seen.insert(rel.clone()) {
                        assets.push(BundleAsset::new(rel, content));
                    }
                    continue;
                }

                if !path.is_dir() {
                    continue;
                }
                let manifest = path.join("SKILL.md");
                if !manifest.is_file() {
                    continue;
                }
                let manifest_content = std::fs::read_to_string(&manifest)?;
                let package_name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                let trigger = parse_frontmatter_value(&manifest_content, "trigger")
                    .map(|v| normalize_trigger_key(&v));
                if !filter_matches_skill(&normalized_filter, &package_name, trigger.as_deref()) {
                    continue;
                }
                collect_files_recursive(&path, skills_dir, &mut assets, &mut seen)?;
            }
        }
        Ok(assets)
    }

    fn collect_workflow_assets(&self, filter: &[&str]) -> Result<Vec<BundleAsset>> {
        let normalized_filter: HashSet<String> = filter
            .iter()
            .map(|s| s.trim().trim_start_matches('/').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let mut assets: Vec<BundleAsset> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for workflows_dir in &self.workflows_dirs {
            if !workflows_dir.exists() {
                continue;
            }
            for entry in std::fs::read_dir(workflows_dir)?.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or_default();
                if ext != "yaml" && ext != "yml" {
                    continue;
                }
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if !normalized_filter.is_empty() && !normalized_filter.contains(&stem) {
                    continue;
                }
                let rel = path
                    .strip_prefix(workflows_dir)
                    .ok()
                    .and_then(|p| p.to_str())
                    .unwrap_or_default()
                    .replace('\\', "/");
                if rel.is_empty() {
                    continue;
                }
                let content = std::fs::read_to_string(&path)?;
                if seen.insert(rel.clone()) {
                    assets.push(BundleAsset::new(rel, content));
                }
            }
        }
        Ok(assets)
    }

    fn collect_tools_assets(
        &self,
        skills: &[BundleAsset],
        workflows: &[BundleAsset],
        tool_filter: &[&str],
    ) -> Result<Vec<BundleAsset>> {
        // "__none__" sentinel → caller explicitly wants no tools.
        if tool_filter.iter().any(|s| s.trim() == "__none__") {
            return Ok(Vec::new());
        }
        let explicit_filter: HashSet<String> = tool_filter
            .iter()
            .map(|s| s.trim().to_ascii_lowercase())
            .map(|s| {
                if let Some(rest) = s.strip_prefix("tools/") {
                    rest.to_string()
                } else {
                    s
                }
            })
            .filter(|s| !s.is_empty())
            .collect();
        if !self.tools_dirs.iter().any(|d| d.exists()) && !self.scripts_dirs.iter().any(|d| d.exists()) {
            return Ok(Vec::new());
        }

        let mut direct_tool_refs: HashSet<String> = HashSet::new();
        let mut selected_skill_triggers: HashSet<String> = HashSet::new();
        let mut workflow_skill_refs: HashSet<String> = HashSet::new();

        for skill in skills {
            collect_tool_refs_from_text(&skill.content, &mut direct_tool_refs);
            if let Some(trigger) = parse_frontmatter_value(&skill.content, "trigger") {
                let key = normalize_trigger_key(&trigger);
                if !key.is_empty() {
                    selected_skill_triggers.insert(key);
                }
            }
        }

        for workflow in workflows {
            collect_tool_refs_from_text(&workflow.content, &mut direct_tool_refs);
            collect_workflow_skill_refs(&workflow.content, &mut workflow_skill_refs);
        }

        // If nothing was auto-detected and user didn't manually select tools,
        // fall back to exporting all discovered tools/scripts. This protects
        // compatibility with stale clients that omit `tools=` in export calls.
        if direct_tool_refs.is_empty()
            && workflow_skill_refs.is_empty()
            && selected_skill_triggers.is_empty()
            && explicit_filter.is_empty()
        {
            return self.collect_all_tool_assets();
        }

        let skill_tool_index = self.build_skill_tool_ref_index()?;
        for trigger in selected_skill_triggers
            .iter()
            .chain(workflow_skill_refs.iter())
        {
            if let Some(refs) = skill_tool_index.get(trigger) {
                for tool in refs {
                    direct_tool_refs.insert(tool.clone());
                }
            }
        }

        // Explicit user selections must be honored even when not referenced.
        // Accept filenames (ml/infer.py), package names (ml), or manifest stems.
        if !explicit_filter.is_empty() {
            for selected in &explicit_filter {
                if selected.is_empty() {
                    continue;
                }
                direct_tool_refs.insert(selected.clone());
                if !selected.contains('/') {
                    direct_tool_refs.insert(format!("{selected}/TOOL.yaml"));
                }
            }
        }

        let mut assets = Vec::new();
        let mut seen = HashSet::new();
        for rel in direct_tool_refs {
            // Named tool reference (e.g. "tool:adb-flash") resolves to tools/<name>/ package.
            if !rel.contains('/') {
                let mut consumed = false;
                for tools_dir in &self.tools_dirs {
                    // 1) Package directory tools/<name>/*
                    let package_dir = tools_dir.join(&rel);
                    if !package_dir.is_dir() || !has_tool_manifest(&package_dir) {
                        // not a package; try next resolution mode
                    } else {
                        let mut package_files = Vec::new();
                        collect_files_recursive_paths(&package_dir, tools_dir, &mut package_files)?;
                        for rel_file in package_files {
                            if seen.contains(&rel_file) {
                                continue;
                            }
                            let package_file = tools_dir.join(&rel_file);
                            if !package_file.is_file() {
                                continue;
                            }
                            let package_content = std::fs::read_to_string(&package_file)?;
                            seen.insert(rel_file.clone());
                            assets.push(BundleAsset::new(rel_file, package_content));
                        }
                        consumed = true;
                        break;
                    }
                }
                if consumed {
                    continue;
                }

                // 2) Flat file tools/<name> (legacy/non-manifest tool file)
                for tools_dir in &self.tools_dirs {
                    let flat_file = tools_dir.join(&rel);
                    if !flat_file.is_file() {
                        continue;
                    }
                    let rel_file = rel.replace('\\', "/");
                    if seen.contains(&rel_file) {
                        consumed = true;
                        break;
                    }
                    let content = std::fs::read_to_string(&flat_file)?;
                    seen.insert(rel_file.clone());
                    assets.push(BundleAsset::new(rel_file, content));
                    consumed = true;
                    break;
                }
                if consumed {
                    continue;
                }

                // 3) Flat script file scripts/<name>
                for scripts_dir in &self.scripts_dirs {
                    let flat_script = scripts_dir.join(&rel);
                    if !flat_script.is_file() {
                        continue;
                    }
                    let rel_file = format!("scripts/{}", rel.replace('\\', "/"));
                    if seen.contains(&rel_file) {
                        break;
                    }
                    let content = std::fs::read_to_string(&flat_script)?;
                    seen.insert(rel_file.clone());
                    assets.push(BundleAsset::new(rel_file, content));
                    break;
                }
                continue;
            }
            let safe_rel = sanitize_relative_reference(&rel)?;
            let rel_norm = safe_rel.to_string_lossy().replace('\\', "/");
            if seen.contains(&rel_norm) {
                continue;
            }

            if let Some(script_rel) = rel_norm.strip_prefix("scripts/") {
                for scripts_dir in &self.scripts_dirs {
                    let path = scripts_dir.join(script_rel);
                    if !path.is_file() {
                        continue;
                    }
                    let content = std::fs::read_to_string(&path)?;
                    seen.insert(rel_norm.clone());
                    assets.push(BundleAsset::new(rel_norm.clone(), content));
                    break;
                }
                if seen.contains(&rel_norm) {
                    continue;
                }
            }

            // Search all tools dirs in order; use the first match.
            for tools_dir in &self.tools_dirs {
                let path = tools_dir.join(&safe_rel);
                if !path.is_file() {
                    continue;
                }
                let content = std::fs::read_to_string(&path)?;
                seen.insert(rel_norm.clone());
                assets.push(BundleAsset::new(rel_norm.clone(), content));
                // If this file is inside a manifest-based tool package (tools/<name>/...),
                // include sibling package files for dependency closure.
                if let Some(package_name) = rel_norm.split('/').next() {
                    if !package_name.is_empty() {
                        let package_dir = tools_dir.join(package_name);
                        if package_dir.is_dir() && has_tool_manifest(&package_dir) {
                            let mut package_files = Vec::new();
                            collect_files_recursive_paths(
                                &package_dir,
                                tools_dir,
                                &mut package_files,
                            )?;
                            for rel_file in package_files {
                                if seen.contains(&rel_file) {
                                    continue;
                                }
                                let package_file = tools_dir.join(&rel_file);
                                if !package_file.is_file() {
                                    continue;
                                }
                                let package_content = std::fs::read_to_string(&package_file)?;
                                seen.insert(rel_file.clone());
                                assets.push(BundleAsset::new(rel_file, package_content));
                            }
                        }
                    }
                }
                break;
            }
        }
        // Apply explicit filter: if non-empty, only keep the listed tools.
        if !explicit_filter.is_empty() {
            assets.retain(|a| {
                let filename = a.filename.to_ascii_lowercase();
                if explicit_filter.contains(&filename) {
                    return true;
                }
                let stem = std::path::Path::new(&a.filename)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if !stem.is_empty() && explicit_filter.contains(&stem) {
                    return true;
                }
                let package = filename.split('/').next().unwrap_or_default();
                !package.is_empty() && explicit_filter.contains(package)
            });
            // Defensive fallback: explicit user selections should never silently
            // produce an empty tools array because of key-shape drift.
            if assets.is_empty() {
                let recovered = self.collect_explicit_tool_assets_fallback(&explicit_filter)?;
                assets.extend(recovered);
            }
        }
        assets.sort_by(|a, b| a.filename.cmp(&b.filename));
        Ok(assets)
    }

    fn collect_all_tool_assets(&self) -> Result<Vec<BundleAsset>> {
        let mut assets = Vec::new();
        let mut seen = HashSet::new();

        // Collect manifest-based tool packages and flat tool files from tools/.
        for tools_dir in &self.tools_dirs {
            if !tools_dir.exists() || !tools_dir.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(tools_dir)?.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if !has_tool_manifest(&path) {
                        continue;
                    }
                    let mut package_files = Vec::new();
                    collect_files_recursive_paths(&path, tools_dir, &mut package_files)?;
                    for rel_file in package_files {
                        if seen.contains(&rel_file) {
                            continue;
                        }
                        let package_file = tools_dir.join(&rel_file);
                        if !package_file.is_file() {
                            continue;
                        }
                        let content = std::fs::read_to_string(&package_file)?;
                        seen.insert(rel_file.clone());
                        assets.push(BundleAsset::new(rel_file, content));
                    }
                } else if path.is_file() {
                    let rel = path
                        .strip_prefix(tools_dir)
                        .ok()
                        .and_then(|p| p.to_str())
                        .unwrap_or_default()
                        .replace('\\', "/");
                    if rel.is_empty() || !seen.insert(rel.clone()) {
                        continue;
                    }
                    let content = std::fs::read_to_string(&path)?;
                    assets.push(BundleAsset::new(rel, content));
                }
            }
        }

        // Collect flat scripts from scripts/.
        for scripts_dir in &self.scripts_dirs {
            if !scripts_dir.exists() || !scripts_dir.is_dir() {
                continue;
            }
            let mut script_paths = Vec::new();
            collect_files_recursive_paths(scripts_dir, scripts_dir, &mut script_paths)?;
            for rel in script_paths {
                let rel_norm = format!("scripts/{}", rel.replace('\\', "/"));
                if !seen.insert(rel_norm.clone()) {
                    continue;
                }
                let path = scripts_dir.join(&rel);
                if !path.is_file() {
                    continue;
                }
                let content = std::fs::read_to_string(&path)?;
                assets.push(BundleAsset::new(rel_norm, content));
            }
        }

        assets.sort_by(|a, b| a.filename.cmp(&b.filename));
        Ok(assets)
    }

    fn collect_explicit_tool_assets_fallback(
        &self,
        explicit_filter: &HashSet<String>,
    ) -> Result<Vec<BundleAsset>> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();

        for selected in explicit_filter {
            if selected.is_empty() {
                continue;
            }

            // Explicit script path: scripts/<name>
            if let Some(script_rel) = selected.strip_prefix("scripts/") {
                for scripts_dir in &self.scripts_dirs {
                    let path = scripts_dir.join(script_rel);
                    if !path.is_file() {
                        continue;
                    }
                    let rel = format!("scripts/{}", script_rel.replace('\\', "/"));
                    if !seen.insert(rel.clone()) {
                        break;
                    }
                    let content = std::fs::read_to_string(&path)?;
                    out.push(BundleAsset::new(rel, content));
                    break;
                }
                continue;
            }

            // Bare path/file lookup in tools/
            for tools_dir in &self.tools_dirs {
                let file_path = tools_dir.join(selected);
                if file_path.is_file() {
                    let rel = selected.replace('\\', "/");
                    if seen.insert(rel.clone()) {
                        let content = std::fs::read_to_string(&file_path)?;
                        out.push(BundleAsset::new(rel, content));
                    }
                    break;
                }
                let package_dir = tools_dir.join(selected);
                if package_dir.is_dir() && has_tool_manifest(&package_dir) {
                    let mut package_files = Vec::new();
                    collect_files_recursive_paths(&package_dir, tools_dir, &mut package_files)?;
                    for rel_file in package_files {
                        if seen.contains(&rel_file) {
                            continue;
                        }
                        let package_file = tools_dir.join(&rel_file);
                        if !package_file.is_file() {
                            continue;
                        }
                        let content = std::fs::read_to_string(&package_file)?;
                        seen.insert(rel_file.clone());
                        out.push(BundleAsset::new(rel_file, content));
                    }
                    break;
                }
            }
        }
        Ok(out)
    }

    fn collect_persona_assets(
        &self,
        skills: &[BundleAsset],
        workflows: &[BundleAsset],
        explicit_filter: &[&str],
    ) -> Result<Vec<BundleAsset>> {
        if !self.persona_dirs.iter().any(|d| d.exists()) {
            return Ok(Vec::new());
        }

        // Explicit selection (lowercased persona names the caller wants regardless of refs).
        let explicit: HashSet<String> = explicit_filter
            .iter()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        // Auto-detect agent names referenced by selected workflows and skills.
        let mut agent_names: HashSet<String> = explicit.clone();
        for workflow in workflows {
            collect_workflow_agent_refs(&workflow.content, &mut agent_names);
        }
        for skill in skills {
            collect_workflow_agent_refs(&skill.content, &mut agent_names);
        }

        if agent_names.is_empty() {
            return Ok(Vec::new());
        }

        let mut assets: Vec<BundleAsset> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for persona_dir in &self.persona_dirs {
            if !persona_dir.exists() {
                continue;
            }
            for entry in std::fs::read_dir(persona_dir)?.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                    continue;
                }
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if !agent_names.contains(&stem) {
                    continue;
                }
                let filename = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                if filename.is_empty() || !seen.insert(filename.clone()) {
                    continue;
                }
                let content = std::fs::read_to_string(&path)?;
                assets.push(BundleAsset::new(filename, content));
            }
        }
        assets.sort_by(|a, b| a.filename.cmp(&b.filename));
        Ok(assets)
    }

    fn build_skill_tool_ref_index(&self) -> Result<HashMap<String, HashSet<String>>> {
        let mut index: HashMap<String, HashSet<String>> = HashMap::new();

        for skills_dir in &self.skills_dirs {
            if !skills_dir.exists() {
                continue;
            }
            for entry in std::fs::read_dir(skills_dir)?.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if path.extension().and_then(|e| e.to_str()) != Some("md") {
                        continue;
                    }
                    let content = std::fs::read_to_string(&path)?;
                    let Some(trigger) = parse_frontmatter_value(&content, "trigger") else {
                        continue;
                    };
                    let key = normalize_trigger_key(&trigger);
                    if key.is_empty() {
                        continue;
                    }
                    let refs = index.entry(key).or_default();
                    collect_tool_refs_from_text(&content, refs);
                    continue;
                }

                if !path.is_dir() {
                    continue;
                }
                let manifest = path.join("SKILL.md");
                if !manifest.is_file() {
                    continue;
                }
                let manifest_content = std::fs::read_to_string(&manifest)?;
                let Some(trigger) = parse_frontmatter_value(&manifest_content, "trigger") else {
                    continue;
                };
                let key = normalize_trigger_key(&trigger);
                if key.is_empty() {
                    continue;
                }
                let refs = index.entry(key).or_default();
                collect_tool_refs_from_text(&manifest_content, refs);
                let mut package_files = Vec::new();
                collect_files_recursive_paths(&path, &path, &mut package_files)?;
                for rel in package_files {
                    collect_tool_refs_from_text(&rel, refs);
                }
            }
        }

        Ok(index)
    }
}

/// Result of importing one asset.
#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub filename: String,
    pub action: ImportAction,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ImportAction {
    Imported,
    Skipped,
    Overwritten,
}

/// Imports a bundle onto disk.
pub struct BundleImporter {
    skills_dir: PathBuf,
    workflows_dir: PathBuf,
    tools_dir: PathBuf,
    personas_dir: PathBuf,
}

impl BundleImporter {
    pub fn new(skills_dir: impl Into<PathBuf>, workflows_dir: impl Into<PathBuf>) -> Self {
        let skills_dir = skills_dir.into();
        let workflows_dir = workflows_dir.into();
        let home = skills_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            skills_dir,
            workflows_dir,
            tools_dir: home.join("tools"),
            personas_dir: home.join("personas"),
        }
    }

    pub fn import(&self, bundle: &Bundle, overwrite: bool) -> Result<Vec<ImportResult>> {
        // verify all hashes first
        for asset in bundle
            .skills
            .iter()
            .chain(bundle.workflows.iter())
            .chain(bundle.tools.iter())
            .chain(bundle.personas.iter())
        {
            if !asset.verify() {
                bail!(
                    "hash mismatch for {}: bundle may be corrupted",
                    asset.filename
                );
            }
        }

        let mut results = Vec::new();
        results.extend(self.write_assets(&bundle.skills, &self.skills_dir, overwrite)?);
        results.extend(self.write_assets(&bundle.workflows, &self.workflows_dir, overwrite)?);
        results.extend(self.write_assets(&bundle.tools, &self.tools_dir, overwrite)?);
        results.extend(self.write_assets(&bundle.personas, &self.personas_dir, overwrite)?);
        Ok(results)
    }

    fn write_assets(
        &self,
        assets: &[BundleAsset],
        dir: &Path,
        overwrite: bool,
    ) -> Result<Vec<ImportResult>> {
        let _ = std::fs::create_dir_all(dir);
        let mut results = Vec::new();
        for asset in assets {
            let safe_name = sanitize_relative_asset_path(&asset.filename)?;
            let dest = dir.join(&safe_name);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let action = if dest.exists() && !overwrite {
                ImportAction::Skipped
            } else if dest.exists() {
                std::fs::write(&dest, &asset.content)?;
                ImportAction::Overwritten
            } else {
                std::fs::write(&dest, &asset.content)?;
                ImportAction::Imported
            };
            results.push(ImportResult {
                filename: safe_name.to_string_lossy().to_string(),
                action,
            });
        }
        Ok(results)
    }
}

fn normalize_trigger_key(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn normalize_skill_filter(filter: &[&str]) -> HashSet<String> {
    filter
        .iter()
        .map(|item| item.trim().to_ascii_lowercase())
        .flat_map(|item| {
            let stripped = item.trim_start_matches('/').to_string();
            if stripped != item {
                vec![item, stripped]
            } else {
                vec![item]
            }
        })
        .filter(|item| !item.is_empty())
        .collect()
}

fn filter_matches_skill(
    filter: &HashSet<String>,
    file_stem_or_dir: &str,
    trigger_key: Option<&str>,
) -> bool {
    if filter.is_empty() {
        return true;
    }
    let stem_key = file_stem_or_dir.trim().to_ascii_lowercase();
    if filter.contains(&stem_key) {
        return true;
    }
    if let Some(trigger_key) = trigger_key {
        return filter.contains(trigger_key);
    }
    false
}

fn parse_frontmatter_value(content: &str, key: &str) -> Option<String> {
    let body = content.strip_prefix("---")?;
    let end = body.find("\n---")?;
    let yaml = &body[..end];
    for line in yaml.lines() {
        let mut split = line.splitn(2, ':');
        let left = split.next()?.trim();
        if left != key {
            continue;
        }
        return split.next().map(|right| right.trim().to_string());
    }
    None
}

fn collect_files_recursive(
    root: &Path,
    base_dir: &Path,
    out: &mut Vec<BundleAsset>,
    seen: &mut HashSet<String>,
) -> Result<()> {
    for entry in std::fs::read_dir(root)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, base_dir, out, seen)?;
            continue;
        }
        let rel = path
            .strip_prefix(base_dir)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or_default()
            .replace('\\', "/");
        if rel.is_empty() || !seen.insert(rel.clone()) {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        out.push(BundleAsset::new(rel, content));
    }
    Ok(())
}

fn collect_files_recursive_paths(
    root: &Path,
    base_dir: &Path,
    out: &mut Vec<String>,
) -> Result<()> {
    for entry in std::fs::read_dir(root)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive_paths(&path, base_dir, out)?;
            continue;
        }
        let rel = path
            .strip_prefix(base_dir)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or_default()
            .replace('\\', "/");
        if !rel.is_empty() {
            out.push(rel);
        }
    }
    Ok(())
}

fn has_tool_manifest(dir: &Path) -> bool {
    [
        "TOOL.yaml",
        "tool.yaml",
        "TOOL.yml",
        "tool.yml",
        "TOOL.toml",
        "tool.toml",
    ]
    .iter()
    .map(|name| dir.join(name))
    .any(|path| path.is_file())
}

fn normalize_reference_token(raw: &str) -> String {
    let trimmed = raw.trim_matches(|c: char| {
        matches!(
            c,
            '`' | '"' | '\'' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
        )
    });
    let without_query = trimmed.split('?').next().unwrap_or(trimmed);
    let without_fragment = without_query.split('#').next().unwrap_or(without_query);
    without_fragment
        .trim_end_matches('.')
        .trim()
        .replace('\\', "/")
}

fn collect_tool_ref_from_token(token: &str, out: &mut HashSet<String>) {
    let normalized = token.trim().trim_start_matches("./");
    if normalized.is_empty() {
        return;
    }

    if let Some(named) = normalized.strip_prefix("tool:") {
        let name = named.trim().trim_matches('"').trim_matches('\'');
        if !name.is_empty() {
            out.insert(name.to_ascii_lowercase());
        }
    }

    if let Some(idx) = normalized.find(".agent007/") {
        let nested = &normalized[idx + ".agent007/".len()..];
        collect_tool_ref_from_token(nested, out);
    }

    if let Some(idx) = normalized.find("tools/") {
        let rel = &normalized[idx + "tools/".len()..];
        let rel = rel.trim_start_matches('/');
        if !rel.is_empty() {
            out.insert(rel.to_string());
        }
    }

    if let Some(idx) = normalized.find("scripts/") {
        let rel = &normalized[idx..];
        let rel = rel.trim_start_matches('/');
        if !rel.is_empty() {
            out.insert(rel.to_string());
        }
    }
}

fn collect_tool_refs_from_text(text: &str, out: &mut HashSet<String>) {
    for raw in text.split_whitespace() {
        let token = normalize_reference_token(raw);
        if token.is_empty() || token.starts_with("http://") || token.starts_with("https://") {
            continue;
        }
        collect_tool_ref_from_token(&token, out);
    }
}

fn collect_workflow_agent_refs(content: &str, out: &mut HashSet<String>) {
    for line in content.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("agent:") {
            continue;
        }
        let value = trimmed
            .trim_start_matches("agent:")
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        let key = value.trim().to_ascii_lowercase();
        if !key.is_empty() {
            out.insert(key);
        }
    }
}

fn collect_workflow_skill_refs(content: &str, out: &mut HashSet<String>) {
    for line in content.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("skill:") {
            continue;
        }
        let value = trimmed
            .trim_start_matches("skill:")
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        let key = normalize_trigger_key(value);
        if !key.is_empty() {
            out.insert(key);
        }
    }
}

fn sanitize_relative_reference(raw: &str) -> Result<PathBuf> {
    let normalized = raw.replace('\\', "/");
    if normalized.starts_with('/') {
        bail!("tool reference must be relative: {}", raw);
    }

    let mut out = PathBuf::new();
    for segment in normalized.split('/') {
        let seg = segment.trim();
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            bail!("tool reference contains parent traversal: {}", raw);
        }
        out.push(seg);
    }

    if out.as_os_str().is_empty() {
        bail!("tool reference is empty after sanitization: {}", raw);
    }

    Ok(out)
}

fn sanitize_relative_asset_path(raw: &str) -> Result<PathBuf> {
    let normalized = raw.replace('\\', "/");
    if normalized.starts_with('/') {
        bail!("asset path must be relative: {}", raw);
    }
    let mut out = PathBuf::new();
    for segment in normalized.split('/') {
        let seg = segment.trim();
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            bail!("asset path contains parent traversal: {}", raw);
        }
        let safe: String = seg
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        if safe.is_empty() {
            continue;
        }
        out.push(safe);
    }
    if out.as_os_str().is_empty() {
        bail!("asset path is empty after sanitization: {}", raw);
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ShareArtifactKind {
    MemoryNote,
    RunLearning,
    EvalSummary,
    Custom,
}

impl ShareArtifactKind {
    fn as_str(&self) -> &'static str {
        match self {
            ShareArtifactKind::MemoryNote => "memory-note",
            ShareArtifactKind::RunLearning => "run-learning",
            ShareArtifactKind::EvalSummary => "eval-summary",
            ShareArtifactKind::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShareArtifact {
    pub kind: ShareArtifactKind,
    pub artifact_id: String,
    pub summary: String,
    pub payload: String,
    #[serde(default)]
    pub labels: Vec<String>,
}

impl ShareArtifact {
    pub fn new(
        kind: ShareArtifactKind,
        artifact_id: impl Into<String>,
        summary: impl Into<String>,
        payload: impl Into<String>,
        labels: Vec<String>,
    ) -> Self {
        Self {
            kind,
            artifact_id: artifact_id.into(),
            summary: summary.into(),
            payload: payload.into(),
            labels,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharingPolicy {
    pub enabled: bool,
    pub allow_memory_notes: bool,
    pub allow_run_learnings: bool,
    pub allow_eval_summaries: bool,
    #[serde(default)]
    pub deny_labels: Vec<String>,
    #[serde(default)]
    pub redact_keys: Vec<String>,
}

impl Default for SharingPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_memory_notes: true,
            allow_run_learnings: true,
            allow_eval_summaries: true,
            deny_labels: Vec::new(),
            redact_keys: Vec::new(),
        }
    }
}

impl SharingPolicy {
    pub fn collaboration_default() -> Self {
        Self {
            enabled: true,
            allow_memory_notes: true,
            allow_run_learnings: true,
            allow_eval_summaries: true,
            deny_labels: Vec::new(),
            redact_keys: vec![
                "token".to_string(),
                "api_key".to_string(),
                "password".to_string(),
                "secret".to_string(),
            ],
        }
    }

    pub fn allows_kind(&self, kind: &ShareArtifactKind) -> bool {
        if !self.enabled {
            return true;
        }
        match kind {
            ShareArtifactKind::MemoryNote => self.allow_memory_notes,
            ShareArtifactKind::RunLearning => self.allow_run_learnings,
            ShareArtifactKind::EvalSummary => self.allow_eval_summaries,
            ShareArtifactKind::Custom => false,
        }
    }

    pub fn filter_artifact(&self, mut artifact: ShareArtifact) -> SharingDecision {
        if !self.enabled {
            return SharingDecision::allow(artifact, false, "sharing-disabled");
        }

        if !self.allows_kind(&artifact.kind) {
            return SharingDecision::block(format!(
                "artifact-kind-not-allowed:{}",
                artifact.kind.as_str()
            ));
        }

        let deny_labels = self
            .deny_labels
            .iter()
            .map(|label| label.trim().to_ascii_lowercase())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>();

        for label in &artifact.labels {
            let normalized = label.trim().to_ascii_lowercase();
            if deny_labels.iter().any(|deny| deny == &normalized) {
                return SharingDecision::block(format!("label-denied:{normalized}"));
            }
        }

        let (payload, redaction_applied) = redact_payload(&artifact.payload, &self.redact_keys);
        artifact.payload = payload;
        SharingDecision::allow(artifact, redaction_applied, "allowed")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharingDecision {
    pub allowed: bool,
    pub reason: String,
    pub redaction_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ShareArtifact>,
}

impl SharingDecision {
    fn allow(artifact: ShareArtifact, redaction_applied: bool, reason: impl Into<String>) -> Self {
        Self {
            allowed: true,
            reason: reason.into(),
            redaction_applied,
            artifact: Some(artifact),
        }
    }

    fn block(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: reason.into(),
            redaction_applied: false,
            artifact: None,
        }
    }
}

fn redact_payload(payload: &str, redact_keys: &[String]) -> (String, bool) {
    let mut redacted = payload.to_string();
    let mut changed = false;
    for key in redact_keys {
        let normalized = key.trim();
        if normalized.is_empty() {
            continue;
        }
        let (updated, updated_changed) = redact_key_values(&redacted, normalized);
        redacted = updated;
        changed |= updated_changed;
    }
    (redacted, changed)
}

fn redact_key_values(input: &str, key: &str) -> (String, bool) {
    let marker = format!("{key}=");
    let marker_len = marker.len();
    let lower_marker = marker.to_ascii_lowercase();
    let lower_input = input.to_ascii_lowercase();

    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;
    let mut changed = false;

    while cursor < input.len() {
        let Some(offset) = lower_input[cursor..].find(&lower_marker) else {
            out.push_str(&input[cursor..]);
            break;
        };

        let start = cursor + offset;
        let value_start = start + marker_len;
        out.push_str(&input[cursor..value_start]);

        let value_end = find_value_end(input, value_start);
        if value_end > value_start {
            out.push_str("[redacted]");
            changed = true;
        }
        cursor = value_end;
    }

    (out, changed)
}

fn find_value_end(input: &str, from: usize) -> usize {
    let bytes = input.as_bytes();
    let mut index = from;
    while index < bytes.len() {
        let b = bytes[index];
        if b.is_ascii_whitespace() || matches!(b, b',' | b';' | b'&' | b'|') {
            break;
        }
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_asset_hash_roundtrip() {
        let asset = BundleAsset::new("test.md", "hello world");
        assert!(asset.verify());
    }

    #[test]
    fn bundle_asset_detects_tamper() {
        let mut asset = BundleAsset::new("test.md", "hello world");
        asset.content = "tampered".to_string();
        assert!(!asset.verify());
    }

    #[test]
    fn bundle_json_roundtrip() {
        let bundle = Bundle::new(
            vec![BundleAsset::new(
                "skill.md",
                "---\ntrigger: /test\n---\nhello",
            )],
            vec![],
            vec![],
            vec![],
        );
        let json = bundle.to_json().unwrap();
        let back = Bundle::from_json(&json).unwrap();
        assert_eq!(back.skills[0].filename, "skill.md");
        assert_eq!(back.version, "1");
    }

    #[test]
    fn importer_skips_existing_when_no_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        let asset = BundleAsset::new("existing.md", "original");
        std::fs::write(skills_dir.join("existing.md"), "original").unwrap();

        let bundle = Bundle::new(vec![asset], vec![], vec![], vec![]);
        let importer = BundleImporter::new(&skills_dir, dir.path().join("workflows"));
        let results = importer.import(&bundle, false).unwrap();
        assert_eq!(results[0].action, ImportAction::Skipped);
    }

    #[test]
    fn importer_preserves_nested_paths() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = Bundle::new(
            vec![
                BundleAsset::new(
                    "review-skill/SKILL.md",
                    "---\nname: review\ndescription: test\ntrigger: /review\n---\nbody",
                ),
                BundleAsset::new(
                    "review-skill/tools/analyze.sh",
                    "#!/usr/bin/env bash\necho ok",
                ),
            ],
            vec![],
            vec![],
            vec![],
        );
        let importer = BundleImporter::new(dir.path().join("skills"), dir.path().join("workflows"));
        let results = importer.import(&bundle, true).unwrap();
        assert_eq!(results.len(), 2);
        assert!(dir
            .path()
            .join("skills")
            .join("review-skill")
            .join("SKILL.md")
            .exists());
        assert!(dir
            .path()
            .join("skills")
            .join("review-skill")
            .join("tools")
            .join("analyze.sh")
            .exists());
    }

    #[test]
    fn importer_restores_project_tools() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = Bundle::new(
            vec![],
            vec![],
            vec![BundleAsset::new(
                "adb/flash.sh",
                "#!/usr/bin/env bash\necho flash",
            )],
            vec![],
        );
        let importer = BundleImporter::new(dir.path().join("skills"), dir.path().join("workflows"));
        importer.import(&bundle, true).unwrap();
        assert!(dir
            .path()
            .join("tools")
            .join("adb")
            .join("flash.sh")
            .exists());
    }

    #[test]
    fn builder_selected_skill_exports_only_that_skill_and_associated_tools() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        let workflows_dir = dir.path().join("workflows");
        let tools_dir = dir.path().join("tools");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::create_dir_all(tools_dir.join("ml")).unwrap();
        std::fs::create_dir_all(tools_dir.join("scripts")).unwrap();

        std::fs::write(
            skills_dir.join("ml-skill.md"),
            "---\nname: ml\ndescription: d\ntrigger: /ml-skill\nmodel: codex\n---\nRun tools/ml/infer.py and scripts/train.py on {{args}}\n",
        )
        .unwrap();
        std::fs::write(
            workflows_dir.join("unselected.yaml"),
            "name: unselected\nsteps:\n  - id: one\n    agent: Coder\n    prompt: hi\n",
        )
        .unwrap();
        std::fs::write(tools_dir.join("ml").join("infer.py"), "print('ok')\n").unwrap();
        std::fs::write(
            tools_dir.join("scripts").join("train.py"),
            "print('train')\n",
        )
        .unwrap();

        let builder = BundleBuilder::new([&skills_dir], [&workflows_dir]);
        let bundle = builder
            .build(&["ml-skill"], &["__none__"], &[], &[])
            .unwrap();

        assert_eq!(bundle.skills.len(), 1, "expected only selected skill");
        assert!(
            bundle.workflows.is_empty(),
            "expected no workflows when explicit none selected"
        );
        assert_eq!(
            bundle.tools.len(),
            2,
            "expected associated tools/scripts to be exported"
        );
        let filenames: HashSet<String> = bundle
            .tools
            .iter()
            .map(|asset| asset.filename.clone())
            .collect();
        assert!(filenames.contains("ml/infer.py"));
        assert!(filenames.contains("scripts/train.py"));
    }

    #[test]
    fn builder_selected_workflow_pulls_tools_from_referenced_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        let workflows_dir = dir.path().join("workflows");
        let tools_dir = dir.path().join("tools");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::create_dir_all(tools_dir.join("ml")).unwrap();

        std::fs::write(
            skills_dir.join("ml-skill.md"),
            "---\nname: ml\ndescription: d\ntrigger: /ml-skill\nmodel: codex\n---\nUse tools/ml/predict.py for inference\n",
        )
        .unwrap();
        std::fs::write(
            workflows_dir.join("train.yaml"),
            "name: train\ndescription: run ml\nsteps:\n  - id: infer\n    agent: Coder\n    skill: /ml-skill\n    output: out\n",
        )
        .unwrap();
        std::fs::write(
            tools_dir.join("ml").join("predict.py"),
            "print('predict')\n",
        )
        .unwrap();

        let builder = BundleBuilder::new([&skills_dir], [&workflows_dir]);
        let bundle = builder.build(&["__none__"], &["train"], &[], &[]).unwrap();

        assert_eq!(
            bundle.skills.len(),
            1,
            "workflow export should include dependent skills"
        );
        assert_eq!(bundle.workflows.len(), 1, "expected selected workflow only");
        assert_eq!(bundle.tools.len(), 1, "expected tool from referenced skill");
        assert_eq!(bundle.tools[0].filename, "ml/predict.py");
    }

    #[test]
    fn builder_manual_tool_selection_exports_even_without_references() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        let workflows_dir = dir.path().join("workflows");
        let tools_dir = dir.path().join("tools");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::create_dir_all(tools_dir.join("ml")).unwrap();

        std::fs::write(
            skills_dir.join("plain.md"),
            "---\nname: plain\ndescription: d\ntrigger: /plain\nmodel: codex\n---\nNo tool refs.\n",
        )
        .unwrap();
        std::fs::write(
            tools_dir.join("ml").join("predict.py"),
            "print('predict')\n",
        )
        .unwrap();

        let builder = BundleBuilder::new([&skills_dir], [&workflows_dir]);
        let bundle = builder
            .build(&["plain"], &["__none__"], &["__none__"], &["ml/predict.py"])
            .unwrap();

        assert_eq!(bundle.skills.len(), 1);
        assert!(bundle.workflows.is_empty());
        assert_eq!(bundle.tools.len(), 1);
        assert_eq!(bundle.tools[0].filename, "ml/predict.py");
    }

    #[test]
    fn builder_exports_scripts_from_home_scripts_dir() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        let workflows_dir = dir.path().join("workflows");
        let scripts_dir = dir.path().join("scripts");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::create_dir_all(&scripts_dir).unwrap();

        std::fs::write(
            skills_dir.join("ml-skill.md"),
            "---\nname: ml\ndescription: d\ntrigger: /ml-skill\nmodel: codex\n---\nRun scripts/ml_monitoring_suite.py on {{args}}\n",
        )
        .unwrap();
        std::fs::write(
            scripts_dir.join("ml_monitoring_suite.py"),
            "print('monitor')\n",
        )
        .unwrap();

        let builder = BundleBuilder::new([&skills_dir], [&workflows_dir]);
        let bundle = builder.build(&["ml-skill"], &["__none__"], &[], &[]).unwrap();
        assert_eq!(bundle.tools.len(), 1);
        assert_eq!(bundle.tools[0].filename, "scripts/ml_monitoring_suite.py");
    }

    #[test]
    fn builder_manual_flat_tool_file_selection_exports_file() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        let workflows_dir = dir.path().join("workflows");
        let tools_dir = dir.path().join("tools");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::create_dir_all(&tools_dir).unwrap();

        std::fs::write(
            skills_dir.join("plain.md"),
            "---\nname: plain\ndescription: d\ntrigger: /plain\nmodel: codex\n---\nNo refs\n",
        )
        .unwrap();
        std::fs::write(tools_dir.join("quick-tool.py"), "print('ok')\n").unwrap();

        let builder = BundleBuilder::new([&skills_dir], [&workflows_dir]);
        let bundle = builder
            .build(&["plain"], &["__none__"], &["__none__"], &["quick-tool.py"])
            .unwrap();

        assert_eq!(bundle.tools.len(), 1);
        assert_eq!(bundle.tools[0].filename, "quick-tool.py");
    }

    #[test]
    fn builder_tools_only_selection_exports_scripts_without_skills_or_workflows() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        let workflows_dir = dir.path().join("workflows");
        let scripts_dir = dir.path().join("scripts");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::create_dir_all(&scripts_dir).unwrap();
        std::fs::write(
            scripts_dir.join("ml_monitoring_suite.py"),
            "print('monitor')\n",
        )
        .unwrap();

        let bundle = BundleBuilder::new([&skills_dir], [&workflows_dir]).build(
            &["__none__"],
            &["__none__"],
            &["__none__"],
            &["scripts/ml_monitoring_suite.py"],
        );

        let bundle = bundle.expect("tools-only selection should succeed");
        assert_eq!(bundle.skills.len(), 0);
        assert_eq!(bundle.workflows.len(), 0);
        assert_eq!(bundle.personas.len(), 0);
        assert_eq!(bundle.tools.len(), 1, "expected script tool to be exported");
        assert_eq!(bundle.tools[0].filename, "scripts/ml_monitoring_suite.py");
    }

    #[test]
    fn builder_tools_only_selection_exports_flat_tool_file_without_skills_or_workflows() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        let workflows_dir = dir.path().join("workflows");
        let tools_dir = dir.path().join("tools");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::create_dir_all(&tools_dir).unwrap();
        std::fs::write(tools_dir.join("ftm_excel_report.py"), "print('ok')\n").unwrap();

        let bundle = BundleBuilder::new([&skills_dir], [&workflows_dir]).build(
            &["__none__"],
            &["__none__"],
            &["__none__"],
            &["ftm_excel_report.py"],
        );

        let bundle = bundle.expect("tools-only selection should succeed");
        assert_eq!(bundle.tools.len(), 1, "expected flat tool file export");
        assert_eq!(bundle.tools[0].filename, "ftm_excel_report.py");
    }

    #[test]
    fn builder_legacy_tools_omitted_by_client_falls_back_to_all_tools() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        let workflows_dir = dir.path().join("workflows");
        let tools_dir = dir.path().join("tools");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::create_dir_all(&tools_dir).unwrap();
        std::fs::write(tools_dir.join("flat_tool.py"), "print('flat')\n").unwrap();
        let pkg = tools_dir.join("pack");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("TOOL.yaml"),
            "name: pack\nruntime: shell\nentrypoint: run.sh\n",
        )
        .unwrap();
        std::fs::write(pkg.join("run.sh"), "#!/usr/bin/env sh\necho ok\n").unwrap();

        // Simulate stale client that sends empty tools selection while also
        // sending explicit empty skills/workflows/personas.
        let bundle = BundleBuilder::new([&skills_dir], [&workflows_dir]).build(
            &["__none__"],
            &["__none__"],
            &["__none__"],
            &[],
        );

        let bundle = bundle.expect("legacy fallback should succeed");
        let names: HashSet<String> = bundle.tools.iter().map(|a| a.filename.clone()).collect();
        assert!(names.contains("flat_tool.py"));
        assert!(names.contains("pack/TOOL.yaml"));
        assert!(names.contains("pack/run.sh"));
    }

    #[test]
    fn builder_workflow_pulls_persona_for_referenced_agent() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        let workflows_dir = dir.path().join("workflows");
        let personas_dir = dir.path().join("personas");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::create_dir_all(&personas_dir).unwrap();

        std::fs::write(
            workflows_dir.join("review.yaml"),
            "name: review\nsteps:\n  - id: s1\n    agent: Architect\n    prompt: design it\n",
        )
        .unwrap();
        std::fs::write(
            personas_dir.join("architect.toml"),
            "[persona]\nname = \"Architect\"\n",
        )
        .unwrap();
        // unrelated persona — should NOT be included
        std::fs::write(
            personas_dir.join("coder.toml"),
            "[persona]\nname = \"Coder\"\n",
        )
        .unwrap();

        let builder =
            BundleBuilder::new([&skills_dir], [&workflows_dir]).with_persona_dirs([&personas_dir]);
        let bundle = builder.build(&[], &["review"], &[], &[]).unwrap();

        assert_eq!(bundle.workflows.len(), 1);
        assert_eq!(
            bundle.personas.len(),
            1,
            "only the referenced persona exported"
        );
        assert_eq!(bundle.personas[0].filename, "architect.toml");
    }

    #[test]
    fn importer_writes_personas() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = Bundle::new(
            vec![],
            vec![],
            vec![],
            vec![BundleAsset::new(
                "architect.toml",
                "[persona]\nname = \"Architect\"\n",
            )],
        );
        let importer = BundleImporter::new(dir.path().join("skills"), dir.path().join("workflows"));
        importer.import(&bundle, true).unwrap();
        assert!(dir.path().join("personas").join("architect.toml").exists());
    }

    #[test]
    fn policy_blocks_disallowed_kind() {
        let policy = SharingPolicy {
            enabled: true,
            allow_memory_notes: true,
            allow_run_learnings: false,
            allow_eval_summaries: true,
            deny_labels: vec![],
            redact_keys: vec![],
        };

        let artifact = ShareArtifact::new(
            ShareArtifactKind::RunLearning,
            "run:42",
            "learning",
            "status=ok",
            vec!["learning".to_string()],
        );

        let decision = policy.filter_artifact(artifact);
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "artifact-kind-not-allowed:run-learning");
        assert!(decision.artifact.is_none());
    }

    #[test]
    fn policy_blocks_denied_label() {
        let policy = SharingPolicy {
            enabled: true,
            allow_memory_notes: true,
            allow_run_learnings: true,
            allow_eval_summaries: true,
            deny_labels: vec!["internal-only".to_string()],
            redact_keys: vec![],
        };

        let artifact = ShareArtifact::new(
            ShareArtifactKind::MemoryNote,
            "memory:1",
            "memory",
            "owner=neo",
            vec!["internal-only".to_string()],
        );

        let decision = policy.filter_artifact(artifact);
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "label-denied:internal-only");
    }

    #[test]
    fn policy_redacts_sensitive_values() {
        let policy = SharingPolicy::collaboration_default();
        let artifact = ShareArtifact::new(
            ShareArtifactKind::MemoryNote,
            "memory:1",
            "memory",
            "owner=neo token=abc123 status=ok",
            vec!["memory".to_string()],
        );

        let decision = policy.filter_artifact(artifact);
        assert!(decision.allowed);
        assert!(decision.redaction_applied);
        let payload = &decision.artifact.expect("artifact").payload;
        assert_eq!(payload, "owner=neo token=[redacted] status=ok");
    }
}
