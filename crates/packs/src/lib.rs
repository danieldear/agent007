//! Optional domain-pack registry and lifecycle management.
//!
//! Packs are versioned `.a7bundle` artifacts installed below an agent007 home.
//! The active version is recorded in `packs/lock.json`; core catalog loaders read
//! that lockfile and add enabled pack directories as overlays.

use agent007_sharing::{Bundle, BundleAsset, BundleImporter};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub const REGISTRY_SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const LOCK_SCHEMA_VERSION: u32 = 1;
const MAX_REGISTRY_BYTES: u64 = 5 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 100 * 1024 * 1024;
pub const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/danieldear/agent007/main/registry/v1/index.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryIndex {
    pub schema_version: u32,
    pub generated_at: String,
    #[serde(default)]
    pub packs: Vec<RegistryPack>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryPack {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub versions: Vec<RegistryPackVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryPackVersion {
    pub version: String,
    pub min_agent007: String,
    pub manifest_url: String,
    pub manifest_sha256: String,
    pub artifact_url: String,
    pub artifact_sha256: String,
    pub size_bytes: u64,
    pub published_at: String,
    #[serde(default)]
    pub yanked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackManifest {
    pub schema_version: u32,
    pub pack: PackMetadata,
    #[serde(default)]
    pub contents: PackContents,
    #[serde(default)]
    pub permissions: PackPermissions,
    #[serde(default)]
    pub dependencies: PackDependencies,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub license: String,
    #[serde(default)]
    pub authors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackContents {
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub workflows: Vec<String>,
    #[serde(default)]
    pub personas: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackPermissions {
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub external_actions: bool,
    #[serde(default)]
    pub approval_required: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackDependencies {
    #[serde(default)]
    pub packs: Vec<PackDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackDependency {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackLock {
    pub schema_version: u32,
    #[serde(default)]
    pub packs: BTreeMap<String, LockedPack>,
}

impl Default for PackLock {
    fn default() -> Self {
        Self {
            schema_version: LOCK_SCHEMA_VERSION,
            packs: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockedPack {
    pub id: String,
    pub version: String,
    pub enabled: bool,
    pub installed_at: String,
    pub registry: String,
    pub artifact_sha256: String,
    pub manifest_sha256: String,
    #[serde(default)]
    pub history: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySnapshot {
    pub index: RegistryIndex,
    pub source: String,
    pub from_cache: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackInstallResult {
    pub id: String,
    pub version: String,
    pub enabled: bool,
    pub install_dir: PathBuf,
    pub dependencies_installed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackInspection {
    pub registry: String,
    pub pack: RegistryPack,
    pub version: RegistryPackVersion,
    pub manifest: PackManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltPackArtifact {
    pub path: PathBuf,
    pub sha256: String,
    pub size_bytes: u64,
    pub skills: usize,
    pub workflows: usize,
    pub personas: usize,
    pub tools: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryVerificationReport {
    pub registry: String,
    pub packs_checked: usize,
    pub versions_checked: usize,
    pub valid: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
struct ResolvedPack {
    registry_pack: RegistryPack,
    version: RegistryPackVersion,
    manifest: PackManifest,
    manifest_text: String,
}

#[derive(Debug, Deserialize)]
struct InstalledPackMetadata {
    id: String,
    version: String,
    artifact_sha256: String,
    manifest_sha256: String,
}

struct PackMutationGuard {
    path: PathBuf,
}

impl Drop for PackMutationGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Registry client and lifecycle manager for one install scope/home.
#[derive(Clone)]
pub struct PackManager {
    home: PathBuf,
    registry: String,
    agent007_version: Version,
    offline: bool,
    allow_external_actions: bool,
    client: reqwest::Client,
}

impl PackManager {
    pub fn new(
        home: impl Into<PathBuf>,
        registry: impl Into<String>,
        agent007_version: &str,
    ) -> Result<Self> {
        let agent007_version = Version::parse(agent007_version)
            .with_context(|| format!("invalid agent007 version '{agent007_version}'"))?;
        Ok(Self {
            home: home.into(),
            registry: registry.into(),
            agent007_version,
            offline: false,
            allow_external_actions: false,
            client: reqwest::Client::builder()
                .user_agent(format!(
                    "agent007/{} pack-manager",
                    env!("CARGO_PKG_VERSION")
                ))
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
        })
    }

    pub fn with_offline(mut self, offline: bool) -> Self {
        self.offline = offline;
        self
    }

    /// Explicitly allow installation of a manifest that declares external
    /// actions. This is opt-in and disabled for Hub lifecycle operations.
    pub fn with_external_actions_allowed(mut self, allowed: bool) -> Self {
        self.allow_external_actions = allowed;
        self
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn lock_path(&self) -> PathBuf {
        self.home.join("packs").join("lock.json")
    }

    pub fn load_lock(&self) -> Result<PackLock> {
        load_lock_from_home(&self.home)
    }

    pub async fn registry(&self, refresh: bool) -> Result<RegistrySnapshot> {
        let cache_path = self.home.join("packs/cache/registry.json");
        if self.offline {
            return read_registry_cache(&cache_path, &self.registry);
        }

        if !refresh && cache_is_fresh(&cache_path, std::time::Duration::from_secs(900)) {
            return read_registry_cache(&cache_path, &self.registry);
        }

        match self
            .fetch_source(&self.registry, None, MAX_REGISTRY_BYTES)
            .await
        {
            Ok(bytes) => {
                let index: RegistryIndex =
                    serde_json::from_slice(&bytes).context("registry index is not valid JSON")?;
                validate_registry(&index)?;
                atomic_write(&cache_path, &bytes)?;
                Ok(RegistrySnapshot {
                    index,
                    source: self.registry.clone(),
                    from_cache: false,
                })
            }
            Err(_fetch_error) if cache_path.is_file() => {
                let mut cached = read_registry_cache(&cache_path, &self.registry)?;
                cached.from_cache = true;
                Ok(cached)
            }
            Err(fetch_error) => Err(fetch_error),
        }
    }

    pub async fn search(&self, query: &str, refresh: bool) -> Result<Vec<RegistryPack>> {
        let snapshot = self.registry(refresh).await?;
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|term| term.to_ascii_lowercase())
            .collect();
        let mut packs: Vec<RegistryPack> = snapshot
            .index
            .packs
            .into_iter()
            .filter(|pack| {
                if terms.is_empty() {
                    return true;
                }
                let haystack = format!(
                    "{} {} {} {} {}",
                    pack.id,
                    pack.name,
                    pack.description,
                    pack.categories.join(" "),
                    pack.tags.join(" ")
                )
                .to_ascii_lowercase();
                terms.iter().all(|term| haystack.contains(term))
            })
            .collect();
        packs.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(packs)
    }

    pub async fn info(&self, id: &str, refresh: bool) -> Result<RegistryPack> {
        validate_pack_id(id)?;
        self.registry(refresh)
            .await?
            .index
            .packs
            .into_iter()
            .find(|pack| pack.id == id)
            .ok_or_else(|| anyhow!("pack '{id}' was not found in the registry"))
    }

    pub async fn inspect(
        &self,
        id: &str,
        version_requirement: Option<&str>,
        refresh: bool,
    ) -> Result<PackInspection> {
        validate_pack_id(id)?;
        let snapshot = self.registry(refresh).await?;
        let pack = snapshot
            .index
            .packs
            .iter()
            .find(|pack| pack.id == id)
            .cloned()
            .ok_or_else(|| anyhow!("pack '{id}' was not found in the registry"))?;
        let requirement = VersionReq::parse(version_requirement.unwrap_or("*"))
            .with_context(|| format!("invalid version requirement for '{id}'"))?;
        let version = select_version(&pack, &requirement, &self.agent007_version)?;
        let manifest = self.fetch_manifest(id, &version).await?;
        Ok(PackInspection {
            registry: snapshot.source,
            pack,
            version,
            manifest,
        })
    }

    pub async fn verify_registry(&self, refresh: bool) -> Result<RegistryVerificationReport> {
        let snapshot = self.registry(refresh).await?;
        let mut report = RegistryVerificationReport {
            registry: snapshot.source.clone(),
            packs_checked: snapshot.index.packs.len(),
            versions_checked: 0,
            valid: true,
            errors: Vec::new(),
        };

        for pack in &snapshot.index.packs {
            for version in &pack.versions {
                report.versions_checked += 1;
                if let Err(error) = self.verify_registry_version(pack, version).await {
                    report
                        .errors
                        .push(format!("{}@{}: {:#}", pack.id, version.version, error));
                }
            }
        }
        report.valid = report.errors.is_empty();
        Ok(report)
    }

    pub async fn install(
        &self,
        id: &str,
        version_requirement: Option<&str>,
        enable: bool,
        refresh: bool,
    ) -> Result<PackInstallResult> {
        let _guard = self.acquire_mutation_lock()?;
        self.install_unlocked(id, version_requirement, enable, refresh)
            .await
    }

    async fn install_unlocked(
        &self,
        id: &str,
        version_requirement: Option<&str>,
        enable: bool,
        refresh: bool,
    ) -> Result<PackInstallResult> {
        validate_pack_id(id)?;
        let snapshot = self.registry(refresh).await?;
        let requirement = VersionReq::parse(version_requirement.unwrap_or("*"))
            .with_context(|| format!("invalid version requirement for '{id}'"))?;
        let resolved = self
            .resolve_install_plan(&snapshot.index, id, &requirement)
            .await?;

        let root_version = resolved
            .iter()
            .find(|pack| pack.registry_pack.id == id)
            .map(|pack| pack.version.version.clone())
            .ok_or_else(|| anyhow!("internal error: root pack was not resolved"))?;

        let mut lock = self.load_lock()?;
        validate_effective_dependencies(&self.home, &lock, &resolved)?;

        let mut installed_ids = Vec::new();
        for pack in &resolved {
            self.materialize(pack).await?;
            installed_ids.push(pack.registry_pack.id.clone());
        }

        for pack in &resolved {
            let pack_id = pack.registry_pack.id.clone();
            let next_version = pack.version.version.clone();
            let existing = lock.packs.get(&pack_id).cloned();
            let mut history = existing
                .as_ref()
                .map(|entry| entry.history.clone())
                .unwrap_or_default();
            if let Some(existing) = &existing {
                if existing.version != next_version && !history.contains(&existing.version) {
                    history.push(existing.version.clone());
                }
            }
            lock.packs.insert(
                pack_id.clone(),
                LockedPack {
                    id: pack_id,
                    version: next_version,
                    enabled: if pack.registry_pack.id == id {
                        enable
                    } else {
                        true
                    },
                    installed_at: Utc::now().to_rfc3339(),
                    registry: snapshot.source.clone(),
                    artifact_sha256: pack.version.artifact_sha256.clone(),
                    manifest_sha256: pack.version.manifest_sha256.clone(),
                    history,
                },
            );
        }
        self.save_lock(&lock)?;

        let dependencies_installed = installed_ids
            .into_iter()
            .filter(|pack_id| pack_id != id)
            .collect();
        Ok(PackInstallResult {
            id: id.to_string(),
            version: root_version.clone(),
            enabled: enable,
            install_dir: pack_version_dir(&self.home, id, &root_version),
            dependencies_installed,
        })
    }

    pub async fn update(&self, id: &str, refresh: bool) -> Result<PackInstallResult> {
        let _guard = self.acquire_mutation_lock()?;
        let enabled = self
            .load_lock()?
            .packs
            .get(id)
            .map(|pack| pack.enabled)
            .ok_or_else(|| anyhow::anyhow!("pack '{id}' is not installed"))?;
        self.install_unlocked(id, None, enabled, refresh).await
    }

    pub fn enable(&self, id: &str) -> Result<LockedPack> {
        let _guard = self.acquire_mutation_lock()?;
        self.set_enabled_unlocked(id, true)
    }

    pub fn disable(&self, id: &str) -> Result<LockedPack> {
        let _guard = self.acquire_mutation_lock()?;
        let lock = self.load_lock()?;
        for (other_id, other) in &lock.packs {
            if other_id == id || !other.enabled {
                continue;
            }
            let manifest = read_installed_manifest(&self.home, other)?;
            if manifest
                .dependencies
                .packs
                .iter()
                .any(|dependency| dependency.id == id)
            {
                bail!("cannot disable '{id}': enabled pack '{other_id}' depends on it");
            }
        }
        self.set_enabled_unlocked(id, false)
    }

    pub fn rollback(&self, id: &str) -> Result<LockedPack> {
        let _guard = self.acquire_mutation_lock()?;
        let mut lock = self.load_lock()?;
        let entry = lock
            .packs
            .get_mut(id)
            .ok_or_else(|| anyhow!("pack '{id}' is not installed"))?;
        let previous = entry
            .history
            .pop()
            .ok_or_else(|| anyhow!("pack '{id}' has no previous version to roll back to"))?;
        let previous_dir = pack_version_dir(&self.home, id, &previous);
        if !previous_dir.is_dir() {
            bail!(
                "cannot roll back '{id}' to {previous}: {} is missing",
                previous_dir.display()
            );
        }
        let metadata = read_install_metadata(&previous_dir)?;
        if metadata.id != id || metadata.version != previous {
            bail!("rollback metadata identity mismatch for '{id}@{previous}'");
        }
        verify_installed_dir(
            &previous_dir,
            &metadata.manifest_sha256,
            &metadata.artifact_sha256,
            None,
        )?;
        let current = std::mem::replace(&mut entry.version, previous);
        entry.artifact_sha256 = metadata.artifact_sha256;
        entry.manifest_sha256 = metadata.manifest_sha256;
        if !entry.history.contains(&current) {
            entry.history.push(current);
        }
        let updated = entry.clone();
        self.save_lock(&lock)?;
        Ok(updated)
    }

    pub fn uninstall(&self, id: &str) -> Result<()> {
        let _guard = self.acquire_mutation_lock()?;
        validate_pack_id(id)?;
        let mut lock = self.load_lock()?;
        if !lock.packs.contains_key(id) {
            bail!("pack '{id}' is not installed");
        }
        for (other_id, other) in &lock.packs {
            if other_id == id {
                continue;
            }
            let manifest = read_installed_manifest(&self.home, other)?;
            if manifest
                .dependencies
                .packs
                .iter()
                .any(|dependency| dependency.id == id)
            {
                bail!("cannot uninstall '{id}': pack '{other_id}' depends on it");
            }
        }
        lock.packs.remove(id);
        self.save_lock(&lock)?;
        let root = self.home.join("packs").join(id);
        if root.exists() {
            fs::remove_dir_all(&root)
                .with_context(|| format!("remove installed pack directory {}", root.display()))?;
        }
        Ok(())
    }

    fn set_enabled_unlocked(&self, id: &str, enabled: bool) -> Result<LockedPack> {
        validate_pack_id(id)?;
        let mut lock = self.load_lock()?;
        let entry = lock
            .packs
            .get_mut(id)
            .ok_or_else(|| anyhow!("pack '{id}' is not installed"))?;
        let version_dir = pack_version_dir(&self.home, id, &entry.version);
        if enabled && !version_dir.is_dir() {
            bail!(
                "installed pack files are missing at {}",
                version_dir.display()
            );
        }
        if enabled {
            verify_installed_dir(
                &version_dir,
                &entry.manifest_sha256,
                &entry.artifact_sha256,
                None,
            )?;
        }
        entry.enabled = enabled;
        let updated = entry.clone();
        self.save_lock(&lock)?;
        Ok(updated)
    }

    fn acquire_mutation_lock(&self) -> Result<PackMutationGuard> {
        let packs_dir = self.home.join("packs");
        fs::create_dir_all(&packs_dir)?;
        let path = packs_dir.join("mutation.lock");
        match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(mut file) => {
                let initialized = (|| -> Result<()> {
                    writeln!(
                        file,
                        "pid={} started_at={}",
                        std::process::id(),
                        Utc::now().to_rfc3339()
                    )?;
                    file.sync_all()?;
                    Ok(())
                })();
                if let Err(error) = initialized {
                    let _ = fs::remove_file(&path);
                    return Err(error).with_context(|| {
                        format!("initialize pack mutation lock {}", path.display())
                    });
                }
                Ok(PackMutationGuard { path })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => bail!(
                "another pack mutation is already in progress; if no agent007 process is changing packs, remove stale lock {}",
                path.display()
            ),
            Err(error) => Err(error)
                .with_context(|| format!("create pack mutation lock {}", path.display())),
        }
    }

    async fn resolve_install_plan(
        &self,
        index: &RegistryIndex,
        root_id: &str,
        root_requirement: &VersionReq,
    ) -> Result<Vec<ResolvedPack>> {
        let mut pending = vec![(root_id.to_string(), root_requirement.clone())];
        let mut resolved: BTreeMap<String, ResolvedPack> = BTreeMap::new();

        while let Some((id, requirement)) = pending.pop() {
            if let Some(existing) = resolved.get(&id) {
                let version = Version::parse(&existing.version.version)?;
                if !requirement.matches(&version) {
                    bail!(
                        "dependency conflict for '{id}': resolved {version}, which does not satisfy {requirement}"
                    );
                }
                continue;
            }

            let registry_pack = index
                .packs
                .iter()
                .find(|pack| pack.id == id)
                .cloned()
                .ok_or_else(|| anyhow!("required pack '{id}' is not present in the registry"))?;
            let version = select_version(&registry_pack, &requirement, &self.agent007_version)?;
            let (manifest, manifest_text) = self.fetch_manifest_text(&id, &version).await?;
            if manifest.permissions.external_actions && !self.allow_external_actions {
                bail!(
                    "pack '{}@{}' declares external actions; review it and retry with explicit approval",
                    id,
                    version.version
                );
            }
            for dependency in &manifest.dependencies.packs {
                let dependency_req = VersionReq::parse(&dependency.version).with_context(|| {
                    format!(
                        "pack '{}@{}' has invalid dependency requirement '{}' for '{}'",
                        id, version.version, dependency.version, dependency.id
                    )
                })?;
                pending.push((dependency.id.clone(), dependency_req));
            }
            resolved.insert(
                id,
                ResolvedPack {
                    registry_pack,
                    version,
                    manifest,
                    manifest_text,
                },
            );
        }

        let mut ordered = Vec::new();
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        visit_pack(
            root_id,
            &resolved,
            &mut visiting,
            &mut visited,
            &mut ordered,
        )?;
        Ok(ordered)
    }

    async fn fetch_manifest(
        &self,
        id: &str,
        version: &RegistryPackVersion,
    ) -> Result<PackManifest> {
        self.fetch_manifest_text(id, version)
            .await
            .map(|(manifest, _)| manifest)
    }

    async fn fetch_manifest_text(
        &self,
        id: &str,
        version: &RegistryPackVersion,
    ) -> Result<(PackManifest, String)> {
        let manifest_bytes = self
            .fetch_source(
                &version.manifest_url,
                Some(&self.registry),
                MAX_MANIFEST_BYTES,
            )
            .await
            .with_context(|| format!("download manifest for '{}@{}'", id, version.version))?;
        verify_sha256(&manifest_bytes, &version.manifest_sha256, "manifest")?;
        let manifest_text = String::from_utf8(manifest_bytes)
            .with_context(|| format!("manifest for '{id}' is not UTF-8"))?;
        let manifest: PackManifest = toml::from_str(&manifest_text)
            .with_context(|| format!("parse manifest for '{}@{}'", id, version.version))?;
        validate_manifest(&manifest)?;
        if manifest.pack.id != id || manifest.pack.version != version.version {
            bail!(
                "manifest identity mismatch: registry selected '{}@{}', manifest declares '{}@{}'",
                id,
                version.version,
                manifest.pack.id,
                manifest.pack.version
            );
        }
        Ok((manifest, manifest_text))
    }

    async fn materialize(&self, resolved: &ResolvedPack) -> Result<()> {
        let id = &resolved.registry_pack.id;
        let version = &resolved.version.version;
        let target = pack_version_dir(&self.home, id, version);
        if target.join("pack.toml").is_file() && target.join("artifact.a7bundle").is_file() {
            if verify_installed_dir(
                &target,
                &resolved.version.manifest_sha256,
                &resolved.version.artifact_sha256,
                Some(resolved.version.size_bytes),
            )
            .is_ok()
            {
                return Ok(());
            }
            fs::remove_dir_all(&target).with_context(|| {
                format!(
                    "remove invalid installed pack directory {}",
                    target.display()
                )
            })?;
        }

        let artifact = self
            .fetch_source(
                &resolved.version.artifact_url,
                Some(&self.registry),
                MAX_ARTIFACT_BYTES,
            )
            .await
            .with_context(|| format!("download artifact for '{id}@{version}'"))?;
        if resolved.version.size_bytes > 0 && artifact.len() as u64 != resolved.version.size_bytes {
            bail!(
                "artifact size mismatch for '{id}@{version}': expected {}, got {}",
                resolved.version.size_bytes,
                artifact.len()
            );
        }
        verify_sha256(&artifact, &resolved.version.artifact_sha256, "artifact")?;
        let bundle_text = String::from_utf8(artifact.clone())
            .with_context(|| format!("artifact for '{id}@{version}' is not a v1 text bundle"))?;
        let bundle = Bundle::from_json(&bundle_text)
            .with_context(|| format!("parse .a7bundle for '{id}@{version}'"))?;

        let staging = self.home.join("packs/.staging").join(format!(
            "{}-{}-{}",
            id,
            version,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&staging)?;
        let importer = BundleImporter::new(staging.join("skills"), staging.join("workflows"));
        if let Err(error) = importer.import(&bundle, true) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error).with_context(|| format!("extract '{id}@{version}'"));
        }
        fs::write(staging.join("pack.toml"), &resolved.manifest_text)?;
        fs::write(staging.join("artifact.a7bundle"), &artifact)?;
        fs::write(
            staging.join("install.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "id": id,
                "version": version,
                "artifact_sha256": resolved.version.artifact_sha256,
                "manifest_sha256": resolved.version.manifest_sha256,
                "installed_at": Utc::now().to_rfc3339(),
            }))?,
        )?;

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if target.exists() {
            fs::remove_dir_all(&target)?;
        }
        fs::rename(&staging, &target)
            .with_context(|| format!("activate staged pack at {}", target.display()))?;
        Ok(())
    }

    async fn verify_registry_version(
        &self,
        pack: &RegistryPack,
        version: &RegistryPackVersion,
    ) -> Result<()> {
        let manifest_bytes = self
            .fetch_source(
                &version.manifest_url,
                Some(&self.registry),
                MAX_MANIFEST_BYTES,
            )
            .await?;
        verify_sha256(&manifest_bytes, &version.manifest_sha256, "manifest")?;
        let manifest_text = String::from_utf8(manifest_bytes)?;
        let manifest: PackManifest = toml::from_str(&manifest_text)?;
        validate_manifest(&manifest)?;
        if manifest.pack.id != pack.id || manifest.pack.version != version.version {
            bail!("manifest identity does not match registry entry");
        }

        let artifact = self
            .fetch_source(
                &version.artifact_url,
                Some(&self.registry),
                MAX_ARTIFACT_BYTES,
            )
            .await?;
        if version.size_bytes > 0 && artifact.len() as u64 != version.size_bytes {
            bail!(
                "artifact size mismatch: expected {}, got {}",
                version.size_bytes,
                artifact.len()
            );
        }
        verify_sha256(&artifact, &version.artifact_sha256, "artifact")?;
        let bundle = Bundle::from_json(&String::from_utf8(artifact)?)?;
        for asset in bundle
            .skills
            .iter()
            .chain(bundle.workflows.iter())
            .chain(bundle.personas.iter())
            .chain(bundle.tools.iter())
        {
            if !asset.verify() {
                bail!(
                    "bundle entry '{}' failed SHA-256 verification",
                    asset.filename
                );
            }
        }
        Ok(())
    }

    fn save_lock(&self, lock: &PackLock) -> Result<()> {
        if lock.schema_version != LOCK_SCHEMA_VERSION {
            bail!(
                "unsupported pack lock schema version {}",
                lock.schema_version
            );
        }
        atomic_write(&self.lock_path(), &serde_json::to_vec_pretty(lock)?)
    }

    async fn fetch_source(
        &self,
        source: &str,
        base: Option<&str>,
        max_bytes: u64,
    ) -> Result<Vec<u8>> {
        let resolved = resolve_source(source, base)?;
        if let Some(path) = resolved.strip_prefix("file://") {
            return read_limited_file(Path::new(path), max_bytes);
        }
        if resolved.starts_with("http://") || resolved.starts_with("https://") {
            let mut response = self
                .client
                .get(&resolved)
                .send()
                .await
                .with_context(|| format!("request '{resolved}'"))?
                .error_for_status()
                .with_context(|| format!("download '{resolved}'"))?;
            if response
                .content_length()
                .is_some_and(|size| size > max_bytes)
            {
                bail!("download '{resolved}' exceeds the {max_bytes}-byte size limit");
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
                    bail!("download '{resolved}' exceeds the {max_bytes}-byte size limit");
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(bytes);
        }
        read_limited_file(Path::new(&resolved), max_bytes)
    }
}

fn read_limited_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let size = fs::metadata(path)
        .with_context(|| format!("stat pack source '{}'", path.display()))?
        .len();
    if size > max_bytes {
        bail!(
            "pack source '{}' exceeds the {max_bytes}-byte size limit",
            path.display()
        );
    }
    fs::read(path).with_context(|| format!("read pack source '{}'", path.display()))
}

pub fn load_lock_from_home(home: &Path) -> Result<PackLock> {
    let path = home.join("packs/lock.json");
    if !path.is_file() {
        return Ok(PackLock::default());
    }
    let lock: PackLock = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("read {}", path.display()))?,
    )
    .with_context(|| format!("parse {}", path.display()))?;
    if lock.schema_version != LOCK_SCHEMA_VERSION {
        bail!(
            "unsupported pack lock schema version {} in {}",
            lock.schema_version,
            path.display()
        );
    }
    Ok(lock)
}

pub fn enabled_pack_roots(home: &Path) -> Vec<PathBuf> {
    let Ok(lock) = load_lock_from_home(home) else {
        return Vec::new();
    };
    lock.packs
        .values()
        .filter(|pack| pack.enabled && locked_pack_has_safe_root_components(pack))
        .map(|pack| pack_version_dir(home, &pack.id, &pack.version))
        .filter(|path| path.is_dir())
        .collect()
}

pub fn pack_version_dir(home: &Path, id: &str, version: &str) -> PathBuf {
    home.join("packs").join(id).join(version)
}

fn locked_pack_has_safe_root_components(pack: &LockedPack) -> bool {
    validate_pack_id(&pack.id).is_ok()
        && Version::parse(&pack.version).is_ok()
        && is_safe_lock_path_component(&pack.id)
        && is_safe_lock_path_component(&pack.version)
}

fn is_safe_lock_path_component(value: &str) -> bool {
    if value.is_empty() || value.contains(['/', '\\']) {
        return false;
    }
    let mut components = Path::new(value).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(component)), None) if component == OsStr::new(value)
    )
}

/// Build a text-safe `.a7bundle` v1 artifact from a pack source directory.
pub fn build_pack_artifact(source_root: &Path, output: &Path) -> Result<BuiltPackArtifact> {
    let manifest_path = source_root.join("pack.toml");
    let manifest_text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: PackManifest = toml::from_str(&manifest_text)
        .with_context(|| format!("parse {}", manifest_path.display()))?;
    validate_manifest(&manifest)?;

    let skills = collect_bundle_assets(&source_root.join("skills"))?;
    let workflows = collect_bundle_assets(&source_root.join("workflows"))?;
    let tools = collect_bundle_assets(&source_root.join("tools"))?;
    let personas = collect_bundle_assets(&source_root.join("personas"))?;
    // Pack artifacts must be byte-for-byte reproducible in CI. The generic
    // sharing bundle records wall-clock export time, so package builds use the
    // conventional deterministic epoch instead.
    let bundle = Bundle {
        version: "1".to_string(),
        created_at: "1970-01-01T00:00:00Z".to_string(),
        skills,
        workflows,
        tools,
        personas,
    };
    let bytes = bundle.to_json()?.into_bytes();
    atomic_write(output, &bytes)?;
    Ok(BuiltPackArtifact {
        path: output.to_path_buf(),
        sha256: hex::encode(Sha256::digest(&bytes)),
        size_bytes: bytes.len() as u64,
        skills: bundle.skills.len(),
        workflows: bundle.workflows.len(),
        personas: bundle.personas.len(),
        tools: bundle.tools.len(),
    })
}

fn collect_bundle_assets(root: &Path) -> Result<Vec<BundleAsset>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_pack_files(root, root, &mut files)?;
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(root)
                .context("pack asset escaped its component root")?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(&path)?;
            let content = String::from_utf8(bytes).with_context(|| {
                format!(
                    "{} is binary or non-UTF-8; .a7bundle v1 supports text assets only",
                    path.display()
                )
            })?;
            Ok(BundleAsset::new(relative, content))
        })
        .collect()
}

fn collect_pack_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("pack source cannot contain symlink '{}'", path.display());
        }
        if metadata.is_dir() {
            collect_pack_files(root, &path, files)?;
        } else if metadata.is_file() {
            path.strip_prefix(root)
                .context("pack source path traversal detected")?;
            files.push(path);
        }
    }
    Ok(())
}

pub fn validate_registry(index: &RegistryIndex) -> Result<()> {
    if index.schema_version != REGISTRY_SCHEMA_VERSION {
        bail!(
            "unsupported registry schema version {}",
            index.schema_version
        );
    }
    let mut ids = BTreeSet::new();
    for pack in &index.packs {
        validate_pack_id(&pack.id)?;
        if !ids.insert(pack.id.clone()) {
            bail!("duplicate pack id '{}'", pack.id);
        }
        if pack.versions.is_empty() {
            bail!("pack '{}' has no published versions", pack.id);
        }
        let mut versions = BTreeSet::new();
        for version in &pack.versions {
            Version::parse(&version.version).with_context(|| {
                format!(
                    "pack '{}' has invalid version '{}'",
                    pack.id, version.version
                )
            })?;
            Version::parse(&version.min_agent007).with_context(|| {
                format!(
                    "pack '{}@{}' has invalid min_agent007 '{}'",
                    pack.id, version.version, version.min_agent007
                )
            })?;
            validate_sha256(&version.manifest_sha256)?;
            validate_sha256(&version.artifact_sha256)?;
            if !versions.insert(version.version.clone()) {
                bail!("pack '{}' repeats version '{}'", pack.id, version.version);
            }
        }
    }
    Ok(())
}

pub fn validate_manifest(manifest: &PackManifest) -> Result<()> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        bail!(
            "unsupported pack manifest schema version {}",
            manifest.schema_version
        );
    }
    validate_pack_id(&manifest.pack.id)?;
    Version::parse(&manifest.pack.version)
        .with_context(|| format!("invalid pack version '{}'", manifest.pack.version))?;
    if manifest.pack.name.trim().is_empty()
        || manifest.pack.description.trim().is_empty()
        || manifest.pack.license.trim().is_empty()
    {
        bail!("pack name, description, and license are required");
    }
    for dependency in &manifest.dependencies.packs {
        validate_pack_id(&dependency.id)?;
        VersionReq::parse(&dependency.version).with_context(|| {
            format!(
                "invalid dependency requirement '{}' for '{}'",
                dependency.version, dependency.id
            )
        })?;
        if dependency.id == manifest.pack.id {
            bail!("pack '{}' cannot depend on itself", manifest.pack.id);
        }
    }
    Ok(())
}

fn select_version(
    pack: &RegistryPack,
    requirement: &VersionReq,
    agent007_version: &Version,
) -> Result<RegistryPackVersion> {
    let mut candidates: Vec<(Version, &RegistryPackVersion)> = pack
        .versions
        .iter()
        .filter(|candidate| !candidate.yanked)
        .filter_map(|candidate| {
            let version = Version::parse(&candidate.version).ok()?;
            let min_agent007 = Version::parse(&candidate.min_agent007).ok()?;
            (requirement.matches(&version) && &min_agent007 <= agent007_version)
                .then_some((version, candidate))
        })
        .collect();
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    candidates
        .pop()
        .map(|(_, version)| version.clone())
        .ok_or_else(|| {
            anyhow!(
                "no compatible version of '{}' satisfies {} for agent007 {}",
                pack.id,
                requirement,
                agent007_version
            )
        })
}

fn visit_pack(
    id: &str,
    resolved: &BTreeMap<String, ResolvedPack>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    ordered: &mut Vec<ResolvedPack>,
) -> Result<()> {
    if visited.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_string()) {
        bail!("pack dependency cycle detected at '{id}'");
    }
    let pack = resolved
        .get(id)
        .ok_or_else(|| anyhow!("resolved dependency '{id}' is missing"))?;
    for dependency in &pack.manifest.dependencies.packs {
        visit_pack(&dependency.id, resolved, visiting, visited, ordered)?;
    }
    visiting.remove(id);
    visited.insert(id.to_string());
    ordered.push(pack.clone());
    Ok(())
}

fn read_installed_manifest(home: &Path, pack: &LockedPack) -> Result<PackManifest> {
    let path = pack_version_dir(home, &pack.id, &pack.version).join("pack.toml");
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("read installed manifest {}", path.display()))?;
    let manifest: PackManifest = toml::from_str(&raw)
        .with_context(|| format!("parse installed manifest {}", path.display()))?;
    validate_manifest(&manifest)
        .with_context(|| format!("validate installed manifest {}", path.display()))?;
    if manifest.pack.id != pack.id || manifest.pack.version != pack.version {
        bail!(
            "installed manifest identity mismatch for '{}@{}'",
            pack.id,
            pack.version
        );
    }
    Ok(manifest)
}

fn read_install_metadata(dir: &Path) -> Result<InstalledPackMetadata> {
    let path = dir.join("install.json");
    serde_json::from_slice(&fs::read(&path).with_context(|| format!("read {}", path.display()))?)
        .with_context(|| format!("parse {}", path.display()))
}

fn verify_installed_dir(
    dir: &Path,
    manifest_sha256: &str,
    artifact_sha256: &str,
    expected_artifact_size: Option<u64>,
) -> Result<()> {
    let manifest_bytes = read_limited_file(&dir.join("pack.toml"), MAX_MANIFEST_BYTES)?;
    verify_sha256(&manifest_bytes, manifest_sha256, "installed manifest")?;
    let manifest: PackManifest = toml::from_str(&String::from_utf8(manifest_bytes)?)?;
    validate_manifest(&manifest)?;

    let artifact = read_limited_file(&dir.join("artifact.a7bundle"), MAX_ARTIFACT_BYTES)?;
    if expected_artifact_size.is_some_and(|size| size > 0 && artifact.len() as u64 != size) {
        bail!("installed artifact size does not match registry metadata");
    }
    verify_sha256(&artifact, artifact_sha256, "installed artifact")?;
    let bundle = Bundle::from_json(&String::from_utf8(artifact)?)?;
    verify_extracted_assets(dir, &bundle)?;
    Ok(())
}

fn verify_extracted_assets(dir: &Path, bundle: &Bundle) -> Result<()> {
    for (kind, assets) in [
        ("skills", &bundle.skills),
        ("workflows", &bundle.workflows),
        ("personas", &bundle.personas),
        ("tools", &bundle.tools),
    ] {
        let root = dir.join(kind);
        let mut expected = BTreeSet::new();
        for asset in assets {
            if !asset.verify() {
                bail!(
                    "installed bundle entry '{}' has an invalid hash",
                    asset.filename
                );
            }
            let relative = safe_asset_path(&asset.filename)?;
            let normalized = relative.to_string_lossy().replace('\\', "/");
            if !expected.insert(normalized.clone()) {
                bail!("installed bundle repeats {kind} asset '{normalized}'");
            }
            let actual = fs::read_to_string(root.join(relative))
                .with_context(|| format!("read extracted {kind} asset '{}'", asset.filename))?;
            if actual != asset.content {
                bail!("extracted {kind} asset '{}' was modified", asset.filename);
            }
        }

        let mut actual = BTreeSet::new();
        if root.exists() {
            let mut files = Vec::new();
            collect_pack_files(&root, &root, &mut files)?;
            for file in files {
                actual.insert(
                    file.strip_prefix(&root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
        if actual != expected {
            bail!("extracted {kind} asset set does not match the verified bundle");
        }
    }
    Ok(())
}

fn safe_asset_path(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("unsafe bundle asset path '{value}'");
    }
    Ok(path.to_path_buf())
}

fn validate_effective_dependencies(
    home: &Path,
    lock: &PackLock,
    resolved: &[ResolvedPack],
) -> Result<()> {
    let mut versions = BTreeMap::new();
    let mut manifests = BTreeMap::new();

    for (id, pack) in &lock.packs {
        versions.insert(
            id.clone(),
            Version::parse(&pack.version)
                .with_context(|| format!("installed pack '{id}' has invalid version"))?,
        );
        manifests.insert(id.clone(), read_installed_manifest(home, pack)?);
    }
    for pack in resolved {
        let id = pack.registry_pack.id.clone();
        versions.insert(id.clone(), Version::parse(&pack.version.version)?);
        manifests.insert(id, pack.manifest.clone());
    }

    for (id, manifest) in manifests {
        for dependency in manifest.dependencies.packs {
            let requirement = VersionReq::parse(&dependency.version)?;
            let installed = versions.get(&dependency.id).ok_or_else(|| {
                anyhow!(
                    "pack '{id}' requires '{} {}', but it would not be installed",
                    dependency.id,
                    requirement
                )
            })?;
            if !requirement.matches(installed) {
                bail!(
                    "dependency conflict: pack '{id}' requires '{} {}', but the effective version would be {}",
                    dependency.id,
                    requirement,
                    installed
                );
            }
        }
    }
    Ok(())
}

fn resolve_source(source: &str, base: Option<&str>) -> Result<String> {
    if source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("file://")
        || Path::new(source).is_absolute()
    {
        return Ok(source.to_string());
    }
    let Some(base) = base else {
        return Ok(source.to_string());
    };
    if base.starts_with("http://") || base.starts_with("https://") {
        return Ok(reqwest::Url::parse(base)?.join(source)?.to_string());
    }
    let base_path = base.strip_prefix("file://").unwrap_or(base);
    let parent = Path::new(base_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    Ok(parent.join(source).to_string_lossy().to_string())
}

fn verify_sha256(bytes: &[u8], expected: &str, kind: &str) -> Result<()> {
    validate_sha256(expected)?;
    let actual = hex::encode(Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("{kind} SHA-256 mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        bail!("invalid SHA-256 digest '{value}'");
    }
    Ok(())
}

fn validate_pack_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 80
        || id.starts_with('-')
        || id.ends_with('-')
        || !id.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        bail!("invalid pack id '{id}': use lowercase kebab-case");
    }
    Ok(())
}

fn cache_is_fresh(path: &Path, ttl: std::time::Duration) -> bool {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
        .map(|age| age <= ttl)
        .unwrap_or(false)
}

fn read_registry_cache(path: &Path, source: &str) -> Result<RegistrySnapshot> {
    let bytes = fs::read(path).with_context(|| {
        format!(
            "registry is unavailable and no cache exists at {}",
            path.display()
        )
    })?;
    let index: RegistryIndex = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse cached registry {}", path.display()))?;
    validate_registry(&index)?;
    Ok(RegistrySnapshot {
        index,
        source: source.to_string(),
        from_cache: true,
    })
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path '{}' has no parent", path.display()))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("pack"),
        uuid::Uuid::new_v4()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent007_sharing::BundleAsset;
    use tempfile::TempDir;

    fn fixture(home: &Path) -> (PathBuf, String) {
        let source = home.join("source");
        fs::create_dir_all(&source).unwrap();
        let manifest = r#"schema_version = 1

[pack]
id = "example-hello"
name = "Example Hello"
version = "1.0.0"
description = "Harmless registry smoke-test pack"
license = "Apache-2.0"
authors = ["agent007"]

[contents]
skills = ["/example-hello"]

[permissions]
network = false
external_actions = false

[dependencies]
packs = []
"#;
        let bundle = Bundle::new(
            vec![BundleAsset::new(
                "example-hello.md",
                "---\nname: Example Hello\ndescription: Harmless test skill\ntrigger: /example-hello\nversion: 1.0.0\n---\nReturn a friendly greeting for {{args}}.\n",
            )],
            vec![],
            vec![],
            vec![],
        );
        let artifact = bundle.to_json().unwrap();
        let manifest_path = source.join("pack.toml");
        let artifact_path = source.join("example-hello.a7bundle");
        fs::write(&manifest_path, manifest).unwrap();
        fs::write(&artifact_path, &artifact).unwrap();
        let manifest_hash = hex::encode(Sha256::digest(manifest.as_bytes()));
        let artifact_hash = hex::encode(Sha256::digest(artifact.as_bytes()));
        let index = RegistryIndex {
            schema_version: 1,
            generated_at: "2026-06-18T00:00:00Z".to_string(),
            packs: vec![RegistryPack {
                id: "example-hello".to_string(),
                name: "Example Hello".to_string(),
                description: "Harmless registry smoke-test pack".to_string(),
                categories: vec!["example".to_string()],
                tags: vec!["smoke-test".to_string()],
                versions: vec![RegistryPackVersion {
                    version: "1.0.0".to_string(),
                    min_agent007: "0.6.0".to_string(),
                    manifest_url: manifest_path.to_string_lossy().to_string(),
                    manifest_sha256: manifest_hash,
                    artifact_url: artifact_path.to_string_lossy().to_string(),
                    artifact_sha256: artifact_hash.clone(),
                    size_bytes: artifact.len() as u64,
                    published_at: "2026-06-18T00:00:00Z".to_string(),
                    yanked: false,
                }],
            }],
        };
        let index_path = source.join("index.json");
        fs::write(&index_path, serde_json::to_vec_pretty(&index).unwrap()).unwrap();
        (index_path, artifact_hash)
    }

    #[tokio::test]
    async fn install_enable_disable_and_uninstall_roundtrip() {
        let dir = TempDir::new().unwrap();
        let (registry, _) = fixture(dir.path());
        let install_home = dir.path().join("home");
        let manager = PackManager::new(
            &install_home,
            registry.to_string_lossy().to_string(),
            "0.6.0",
        )
        .unwrap();

        let result = manager
            .install("example-hello", Some("=1.0.0"), true, true)
            .await
            .unwrap();
        assert!(result.install_dir.join("skills/example-hello.md").is_file());
        assert!(manager.load_lock().unwrap().packs["example-hello"].enabled);
        assert_eq!(enabled_pack_roots(&install_home), vec![result.install_dir]);

        manager.disable("example-hello").unwrap();
        assert!(enabled_pack_roots(&install_home).is_empty());
        manager.enable("example-hello").unwrap();
        assert_eq!(enabled_pack_roots(&install_home).len(), 1);

        manager.uninstall("example-hello").unwrap();
        assert!(!install_home.join("packs/example-hello").exists());
        assert!(manager.load_lock().unwrap().packs.is_empty());
    }

    #[test]
    fn enabled_pack_roots_ignores_tampered_lock_path_components() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let good_root = pack_version_dir(&home, "lock-good", "1.0.0");
        fs::create_dir_all(&good_root).unwrap();
        fs::create_dir_all(home.join("escaped/1.0.0")).unwrap();
        fs::create_dir_all(home.join("packs/evil")).unwrap();

        let locked = |id: &str, version: &str| LockedPack {
            id: id.to_string(),
            version: version.to_string(),
            enabled: true,
            installed_at: Utc::now().to_rfc3339(),
            registry: "fixture".to_string(),
            artifact_sha256: "a".repeat(64),
            manifest_sha256: "b".repeat(64),
            history: vec![],
        };

        let mut lock = PackLock::default();
        lock.packs
            .insert("bad-id".to_string(), locked("../escaped", "1.0.0"));
        lock.packs.insert(
            "bad-version".to_string(),
            locked("example-hello", "../evil"),
        );
        lock.packs
            .insert("valid".to_string(), locked("lock-good", "1.0.0"));

        fs::create_dir_all(home.join("packs")).unwrap();
        fs::write(
            home.join("packs/lock.json"),
            serde_json::to_vec_pretty(&lock).unwrap(),
        )
        .unwrap();

        assert_eq!(enabled_pack_roots(&home), vec![good_root]);
    }

    #[tokio::test]
    async fn enabling_rejects_modified_extracted_assets() {
        let dir = TempDir::new().unwrap();
        let (registry, _) = fixture(dir.path());
        let home = dir.path().join("home");
        let manager = PackManager::new(&home, registry.to_string_lossy(), "0.6.0").unwrap();
        let installed = manager
            .install("example-hello", None, true, true)
            .await
            .unwrap();
        manager.disable("example-hello").unwrap();
        fs::write(
            installed.install_dir.join("skills/example-hello.md"),
            "modified",
        )
        .unwrap();

        let error = manager.enable("example-hello").unwrap_err().to_string();
        assert!(error.contains("was modified"));
    }

    #[test]
    fn concurrent_mutations_fail_without_overwriting_the_lock() {
        let dir = TempDir::new().unwrap();
        let manager = PackManager::new(dir.path(), "registry.json", "0.6.0").unwrap();
        let guard = manager.acquire_mutation_lock().unwrap();
        let error = manager.enable("example-hello").unwrap_err().to_string();
        assert!(error.contains("another pack mutation"));
        drop(guard);
        assert!(!dir.path().join("packs/mutation.lock").exists());
    }

    #[tokio::test]
    async fn rejects_tampered_artifact() {
        let dir = TempDir::new().unwrap();
        let (registry, _) = fixture(dir.path());
        fs::write(dir.path().join("source/example-hello.a7bundle"), "tampered").unwrap();
        let manager = PackManager::new(
            dir.path().join("home"),
            registry.to_string_lossy().to_string(),
            "0.6.0",
        )
        .unwrap();
        let error = manager
            .install("example-hello", None, true, true)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("size mismatch") || error.contains("SHA-256 mismatch"));
    }

    #[tokio::test]
    async fn rejects_tampered_manifest_before_parsing() {
        let dir = TempDir::new().unwrap();
        let (registry, _) = fixture(dir.path());
        fs::write(
            dir.path().join("source/pack.toml"),
            "not the signed manifest",
        )
        .unwrap();
        let manager =
            PackManager::new(dir.path().join("home"), registry.to_string_lossy(), "0.6.0").unwrap();
        let error = manager
            .install("example-hello", None, true, true)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("manifest SHA-256 mismatch"));
    }

    #[tokio::test]
    async fn cached_registry_supports_explicit_offline_search() {
        let dir = TempDir::new().unwrap();
        let (registry, _) = fixture(dir.path());
        let home = dir.path().join("home");
        let manager = PackManager::new(&home, registry.to_string_lossy(), "0.6.0").unwrap();
        assert_eq!(manager.search("hello", true).await.unwrap().len(), 1);
        fs::remove_file(&registry).unwrap();

        let offline = PackManager::new(&home, registry.to_string_lossy(), "0.6.0")
            .unwrap()
            .with_offline(true);
        let snapshot = offline.registry(false).await.unwrap();
        assert!(snapshot.from_cache);
        assert_eq!(snapshot.index.packs[0].id, "example-hello");
    }

    #[tokio::test]
    async fn update_preserves_previous_version_for_rollback() {
        let dir = TempDir::new().unwrap();
        let (registry, _) = fixture(dir.path());
        let home = dir.path().join("home");
        let manager = PackManager::new(&home, registry.to_string_lossy(), "0.6.0").unwrap();
        manager
            .install("example-hello", Some("=1.0.0"), true, true)
            .await
            .unwrap();
        let original_manifest_sha = manager.load_lock().unwrap().packs["example-hello"]
            .manifest_sha256
            .clone();

        let mut index: RegistryIndex =
            serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
        let source = registry.parent().unwrap();
        let manifest = fs::read_to_string(source.join("pack.toml"))
            .unwrap()
            .replace("version = \"1.0.0\"", "version = \"1.1.0\"");
        let manifest_path = source.join("pack-1.1.0.toml");
        fs::write(&manifest_path, &manifest).unwrap();
        let previous = index.packs[0].versions[0].clone();
        index.packs[0].versions.push(RegistryPackVersion {
            version: "1.1.0".to_string(),
            manifest_url: manifest_path.to_string_lossy().to_string(),
            manifest_sha256: hex::encode(Sha256::digest(manifest.as_bytes())),
            published_at: "2026-06-18T01:00:00Z".to_string(),
            ..previous
        });
        fs::write(&registry, serde_json::to_vec_pretty(&index).unwrap()).unwrap();

        let updated = manager.update("example-hello", true).await.unwrap();
        assert_eq!(updated.version, "1.1.0");
        assert_eq!(
            manager.load_lock().unwrap().packs["example-hello"].history,
            vec!["1.0.0"]
        );
        let rolled_back = manager.rollback("example-hello").unwrap();
        assert_eq!(rolled_back.version, "1.0.0");
        assert_eq!(rolled_back.manifest_sha256, original_manifest_sha);
    }

    #[tokio::test]
    async fn update_preserves_disabled_state() {
        let dir = TempDir::new().unwrap();
        let (registry, _) = fixture(dir.path());
        let home = dir.path().join("home");
        let manager = PackManager::new(&home, registry.to_string_lossy(), "0.6.0").unwrap();
        manager
            .install("example-hello", Some("=1.0.0"), false, true)
            .await
            .unwrap();

        let updated = manager.update("example-hello", true).await.unwrap();
        assert!(!updated.enabled);
        assert!(!manager.load_lock().unwrap().packs["example-hello"].enabled);
    }

    #[tokio::test]
    async fn external_actions_require_explicit_install_approval() {
        let dir = TempDir::new().unwrap();
        let (registry, _) = fixture(dir.path());
        let source = registry.parent().unwrap();
        let manifest_path = source.join("pack.toml");
        let manifest = fs::read_to_string(&manifest_path)
            .unwrap()
            .replace("external_actions = false", "external_actions = true");
        fs::write(&manifest_path, &manifest).unwrap();
        let mut index: RegistryIndex =
            serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
        index.packs[0].versions[0].manifest_sha256 =
            hex::encode(Sha256::digest(manifest.as_bytes()));
        fs::write(&registry, serde_json::to_vec_pretty(&index).unwrap()).unwrap();

        let manager = PackManager::new(
            dir.path().join("blocked-home"),
            registry.to_string_lossy(),
            "0.6.0",
        )
        .unwrap();
        assert!(manager
            .install("example-hello", None, true, true)
            .await
            .unwrap_err()
            .to_string()
            .contains("external actions"));

        let approved = PackManager::new(
            dir.path().join("approved-home"),
            registry.to_string_lossy(),
            "0.6.0",
        )
        .unwrap()
        .with_external_actions_allowed(true);
        assert!(approved
            .install("example-hello", None, true, true)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn required_dependency_cannot_be_disabled_or_uninstalled() {
        let dir = TempDir::new().unwrap();
        let (registry, _) = fixture(dir.path());
        let home = dir.path().join("home");
        let manager = PackManager::new(&home, registry.to_string_lossy(), "0.6.0").unwrap();
        manager
            .install("example-hello", None, true, true)
            .await
            .unwrap();

        let dependent_dir = pack_version_dir(&home, "dependent", "1.0.0");
        fs::create_dir_all(&dependent_dir).unwrap();
        fs::write(
            dependent_dir.join("pack.toml"),
            r#"schema_version = 1
[pack]
id = "dependent"
name = "Dependent"
version = "1.0.0"
description = "Depends on example"
license = "Apache-2.0"
[dependencies]
packs = [{ id = "example-hello", version = "^1" }]
"#,
        )
        .unwrap();
        let mut lock = manager.load_lock().unwrap();
        lock.packs.insert(
            "dependent".to_string(),
            LockedPack {
                id: "dependent".to_string(),
                version: "1.0.0".to_string(),
                enabled: true,
                installed_at: Utc::now().to_rfc3339(),
                registry: "fixture".to_string(),
                artifact_sha256: "a".repeat(64),
                manifest_sha256: "b".repeat(64),
                history: vec![],
            },
        );
        manager.save_lock(&lock).unwrap();

        assert!(manager.disable("example-hello").is_err());
        assert!(manager.uninstall("example-hello").is_err());
    }

    #[tokio::test]
    async fn update_rejects_conflicts_with_installed_dependents() {
        let dir = TempDir::new().unwrap();
        let (registry, _) = fixture(dir.path());
        let home = dir.path().join("home");
        let manager = PackManager::new(&home, registry.to_string_lossy(), "0.6.0").unwrap();
        manager
            .install("example-hello", Some("=1.0.0"), true, true)
            .await
            .unwrap();

        let dependent_dir = pack_version_dir(&home, "dependent", "1.0.0");
        fs::create_dir_all(&dependent_dir).unwrap();
        fs::write(
            dependent_dir.join("pack.toml"),
            r#"schema_version = 1
[pack]
id = "dependent"
name = "Dependent"
version = "1.0.0"
description = "Depends on example v1"
license = "Apache-2.0"
[dependencies]
packs = [{ id = "example-hello", version = "^1" }]
"#,
        )
        .unwrap();
        let mut lock = manager.load_lock().unwrap();
        lock.packs.insert(
            "dependent".to_string(),
            LockedPack {
                id: "dependent".to_string(),
                version: "1.0.0".to_string(),
                enabled: true,
                installed_at: Utc::now().to_rfc3339(),
                registry: "fixture".to_string(),
                artifact_sha256: "a".repeat(64),
                manifest_sha256: "b".repeat(64),
                history: vec![],
            },
        );
        manager.save_lock(&lock).unwrap();

        let source = registry.parent().unwrap();
        let manifest = fs::read_to_string(source.join("pack.toml"))
            .unwrap()
            .replace("version = \"1.0.0\"", "version = \"2.0.0\"");
        let manifest_path = source.join("pack-2.0.0.toml");
        fs::write(&manifest_path, &manifest).unwrap();
        let mut index: RegistryIndex =
            serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
        let previous = index.packs[0].versions[0].clone();
        index.packs[0].versions.push(RegistryPackVersion {
            version: "2.0.0".to_string(),
            manifest_url: manifest_path.to_string_lossy().to_string(),
            manifest_sha256: hex::encode(Sha256::digest(manifest.as_bytes())),
            published_at: "2026-06-18T02:00:00Z".to_string(),
            ..previous
        });
        fs::write(&registry, serde_json::to_vec_pretty(&index).unwrap()).unwrap();

        let error = manager
            .update("example-hello", true)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("dependency conflict"));
        assert_eq!(
            manager.load_lock().unwrap().packs["example-hello"].version,
            "1.0.0"
        );
    }

    #[test]
    fn registry_rejects_duplicate_ids() {
        let pack = RegistryPack {
            id: "dup".to_string(),
            name: "Dup".to_string(),
            description: "duplicate".to_string(),
            categories: vec![],
            tags: vec![],
            versions: vec![RegistryPackVersion {
                version: "1.0.0".to_string(),
                min_agent007: "0.6.0".to_string(),
                manifest_url: "manifest.toml".to_string(),
                manifest_sha256: "a".repeat(64),
                artifact_url: "artifact.a7bundle".to_string(),
                artifact_sha256: "b".repeat(64),
                size_bytes: 1,
                published_at: "2026-06-18T00:00:00Z".to_string(),
                yanked: false,
            }],
        };
        let index = RegistryIndex {
            schema_version: 1,
            generated_at: "2026-06-18T00:00:00Z".to_string(),
            packs: vec![pack.clone(), pack],
        };
        assert!(validate_registry(&index)
            .unwrap_err()
            .to_string()
            .contains("duplicate pack id"));
    }

    #[test]
    fn dependency_cycle_is_rejected() {
        fn resolved(id: &str, dependency: &str) -> ResolvedPack {
            let version = RegistryPackVersion {
                version: "1.0.0".to_string(),
                min_agent007: "0.6.0".to_string(),
                manifest_url: "manifest".to_string(),
                manifest_sha256: "a".repeat(64),
                artifact_url: "artifact".to_string(),
                artifact_sha256: "b".repeat(64),
                size_bytes: 1,
                published_at: "2026-06-18T00:00:00Z".to_string(),
                yanked: false,
            };
            ResolvedPack {
                registry_pack: RegistryPack {
                    id: id.to_string(),
                    name: id.to_string(),
                    description: id.to_string(),
                    categories: vec![],
                    tags: vec![],
                    versions: vec![version.clone()],
                },
                version,
                manifest: PackManifest {
                    schema_version: 1,
                    pack: PackMetadata {
                        id: id.to_string(),
                        name: id.to_string(),
                        version: "1.0.0".to_string(),
                        description: id.to_string(),
                        license: "Apache-2.0".to_string(),
                        authors: vec![],
                    },
                    contents: PackContents::default(),
                    permissions: PackPermissions::default(),
                    dependencies: PackDependencies {
                        packs: vec![PackDependency {
                            id: dependency.to_string(),
                            version: "^1".to_string(),
                        }],
                    },
                },
                manifest_text: String::new(),
            }
        }

        let packs = BTreeMap::from([
            ("alpha".to_string(), resolved("alpha", "beta")),
            ("beta".to_string(), resolved("beta", "alpha")),
        ]);
        let error = visit_pack(
            "alpha",
            &packs,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            &mut Vec::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("dependency cycle"));
    }
}
