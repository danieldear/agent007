use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use agent007_core::PersonaSpec;
use agent007_skills::types::SkillFrontmatter;
use agent007_workflows::{dag::DagValidator, types::WorkflowDef};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_ASSET_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Skill,
    Workflow,
    Persona,
}

impl AssetKind {
    pub fn parse(value: &str) -> Result<Self, AssetError> {
        match value {
            "skill" | "skills" => Ok(Self::Skill),
            "workflow" | "workflows" => Ok(Self::Workflow),
            "persona" | "personas" => Ok(Self::Persona),
            _ => Err(AssetError::BadRequest(format!(
                "unsupported asset kind '{value}'"
            ))),
        }
    }

    pub fn plural(self) -> &'static str {
        match self {
            Self::Skill => "skills",
            Self::Workflow => "workflows",
            Self::Persona => "personas",
        }
    }

    fn default_extension(self) -> &'static str {
        match self {
            Self::Skill => "md",
            Self::Workflow => "yaml",
            Self::Persona => "toml",
        }
    }

    fn supports_extension(self, extension: &str) -> bool {
        match self {
            Self::Skill => extension == "md",
            Self::Workflow => matches!(extension, "yaml" | "yml" | "toml"),
            Self::Persona => extension == "toml",
        }
    }
}

impl fmt::Display for AssetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Skill => "skill",
            Self::Workflow => "workflow",
            Self::Persona => "persona",
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VersionBump {
    None,
    Patch,
    Minor,
    Major,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub format: String,
    pub revision: String,
    pub source: &'static str,
    pub editable: bool,
    pub valid: bool,
    pub validation_errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetDocument {
    #[serde(flatten)]
    pub summary: AssetSummary,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug)]
pub enum AssetError {
    BadRequest(String),
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    TooLarge(String),
    Invalid(Vec<String>),
    Io(String),
}

impl fmt::Display for AssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(message)
            | Self::Forbidden(message)
            | Self::NotFound(message)
            | Self::Conflict(message)
            | Self::TooLarge(message)
            | Self::Io(message) => formatter.write_str(message),
            Self::Invalid(errors) => formatter.write_str(&errors.join("; ")),
        }
    }
}

impl std::error::Error for AssetError {}

#[derive(Debug, Clone)]
pub struct GlobalAssetStore {
    global_home: PathBuf,
}

impl GlobalAssetStore {
    pub fn new(global_home: impl Into<PathBuf>) -> Self {
        Self {
            global_home: global_home.into(),
        }
    }

    pub fn list(&self, kind: AssetKind) -> Result<Vec<AssetSummary>, AssetError> {
        let root = self.root(kind);
        if !root.exists() {
            return Ok(Vec::new());
        }
        ensure_safe_directory(&root)?;
        let mut assets = fs::read_dir(&root)
            .map_err(|error| io_error("read global asset directory", error))?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                if !file_type.is_file() || file_type.is_symlink() {
                    return None;
                }
                let extension = entry
                    .path()
                    .extension()?
                    .to_string_lossy()
                    .to_ascii_lowercase();
                kind.supports_extension(&extension).then_some(entry.path())
            })
            .filter_map(|path| self.read_document(kind, path).ok().map(|doc| doc.summary))
            .collect::<Vec<_>>();
        assets.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        Ok(assets)
    }

    pub fn get(&self, kind: AssetKind, id: &str) -> Result<AssetDocument, AssetError> {
        validate_id(id)?;
        let path = self.find_existing(kind, id)?;
        self.read_document(kind, path)
    }

    pub fn validate(
        &self,
        kind: AssetKind,
        format: Option<&str>,
        content: &str,
    ) -> ValidationResult {
        let format = match normalize_format(kind, format) {
            Ok(format) => format,
            Err(error) => {
                return ValidationResult {
                    valid: false,
                    errors: vec![error.to_string()],
                    name: None,
                    description: None,
                    version: None,
                }
            }
        };
        match validate_content(kind, &format, content) {
            Ok(identity) => ValidationResult {
                valid: true,
                errors: Vec::new(),
                name: Some(identity.name),
                description: Some(identity.description),
                version: Some(identity.version),
            },
            Err(error) => ValidationResult {
                valid: false,
                errors: match error {
                    AssetError::Invalid(errors) => errors,
                    other => vec![other.to_string()],
                },
                name: None,
                description: None,
                version: None,
            },
        }
    }

    pub fn create(
        &self,
        kind: AssetKind,
        id: &str,
        format: Option<&str>,
        content: &str,
    ) -> Result<AssetDocument, AssetError> {
        validate_id(id)?;
        validate_size(content)?;
        let format = normalize_format(kind, format)?;
        let root = self.root(kind);
        ensure_writable_directory(&root)?;
        if self.find_existing_optional(kind, id)?.is_some() {
            return Err(AssetError::Conflict(format!(
                "{kind} '{id}' already exists"
            )));
        }
        let content = set_version(kind, &format, content, "1.0.0")?;
        validate_content(kind, &format, &content)?;
        let path = root.join(format!("{id}.{format}"));
        atomic_write(&path, content.as_bytes())?;
        self.read_document(kind, path)
    }

    pub fn update(
        &self,
        kind: AssetKind,
        id: &str,
        expected_revision: &str,
        bump: VersionBump,
        content: &str,
    ) -> Result<AssetDocument, AssetError> {
        validate_id(id)?;
        validate_revision(expected_revision)?;
        validate_size(content)?;
        let path = self.find_existing(kind, id)?;
        let current = self.read_document(kind, path.clone())?;
        if current.summary.revision != expected_revision {
            return Err(AssetError::Conflict(
                "asset changed since it was opened; reload before saving".to_string(),
            ));
        }
        let next_version = bump_version(&current.summary.version, bump)?;
        let content = set_version(kind, &current.summary.format, content, &next_version)?;
        validate_content(kind, &current.summary.format, &content)?;
        self.backup(kind, id, &path, &current.summary.version)?;
        atomic_write(&path, content.as_bytes())?;
        self.read_document(kind, path)
    }

    pub fn delete(
        &self,
        kind: AssetKind,
        id: &str,
        expected_revision: &str,
        confirmation: &str,
    ) -> Result<(), AssetError> {
        validate_id(id)?;
        validate_revision(expected_revision)?;
        if confirmation != id {
            return Err(AssetError::BadRequest(format!(
                "confirmation must exactly match '{id}'"
            )));
        }
        let path = self.find_existing(kind, id)?;
        let current = self.read_document(kind, path.clone())?;
        if current.summary.revision != expected_revision {
            return Err(AssetError::Conflict(
                "asset changed since it was opened; reload before deleting".to_string(),
            ));
        }
        let trash = self.global_home.join("trash/hub").join(kind.plural());
        ensure_writable_directory(&trash)?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or(kind.default_extension());
        let destination = trash.join(format!(
            "{id}-{}-{}.{}",
            Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
            Uuid::new_v4(),
            extension
        ));
        fs::rename(&path, &destination)
            .map_err(|error| io_error("move asset to recoverable trash", error))?;
        Ok(())
    }

    fn root(&self, kind: AssetKind) -> PathBuf {
        self.global_home.join(kind.plural())
    }

    fn find_existing(&self, kind: AssetKind, id: &str) -> Result<PathBuf, AssetError> {
        self.find_existing_optional(kind, id)?
            .ok_or_else(|| AssetError::NotFound(format!("{kind} '{id}' was not found")))
    }

    fn find_existing_optional(
        &self,
        kind: AssetKind,
        id: &str,
    ) -> Result<Option<PathBuf>, AssetError> {
        let root = self.root(kind);
        if !root.exists() {
            return Ok(None);
        }
        ensure_safe_directory(&root)?;
        let mut matches = fs::read_dir(&root)
            .map_err(|error| io_error("read global asset directory", error))?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                if !file_type.is_file() || file_type.is_symlink() {
                    return None;
                }
                let path = entry.path();
                let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
                let stem = path.file_stem()?.to_str()?;
                (stem == id && kind.supports_extension(&extension)).then_some(path)
            })
            .collect::<Vec<_>>();
        matches.sort();
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.pop()),
            _ => Err(AssetError::Conflict(format!(
                "multiple files map to {kind} id '{id}'"
            ))),
        }
    }

    fn read_document(&self, kind: AssetKind, path: PathBuf) -> Result<AssetDocument, AssetError> {
        ensure_safe_file(&path)?;
        let metadata =
            fs::metadata(&path).map_err(|error| io_error("inspect global asset", error))?;
        if metadata.len() > MAX_ASSET_BYTES as u64 {
            return Err(AssetError::TooLarge(format!(
                "asset exceeds the {} byte limit",
                MAX_ASSET_BYTES
            )));
        }
        let content =
            fs::read_to_string(&path).map_err(|error| io_error("read global asset", error))?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or(kind.default_extension())
            .to_ascii_lowercase();
        let id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| AssetError::Forbidden("asset filename is not valid UTF-8".to_string()))?
            .to_string();
        let (identity, valid, validation_errors) =
            match validate_content(kind, &extension, &content) {
                Ok(identity) => (identity, true, Vec::new()),
                Err(error) => {
                    let errors = match error {
                        AssetError::Invalid(errors) => errors,
                        other => vec![other.to_string()],
                    };
                    (
                        fallback_identity(kind, &extension, &content, &id),
                        false,
                        errors,
                    )
                }
            };
        Ok(AssetDocument {
            summary: AssetSummary {
                id,
                name: identity.name,
                description: identity.description,
                version: identity.version,
                format: extension,
                revision: revision(content.as_bytes()),
                source: "global",
                editable: true,
                valid,
                validation_errors,
            },
            content,
        })
    }

    fn backup(
        &self,
        kind: AssetKind,
        id: &str,
        source: &Path,
        version: &str,
    ) -> Result<(), AssetError> {
        let backup = self.global_home.join("backups/hub").join(kind.plural());
        ensure_writable_directory(&backup)?;
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or(kind.default_extension());
        let destination = backup.join(format!(
            "{id}-v{version}-{}-{}.{}",
            Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
            Uuid::new_v4(),
            extension
        ));
        fs::copy(source, destination)
            .map(|_| ())
            .map_err(|error| io_error("back up global asset", error))
    }
}

#[derive(Debug)]
struct AssetIdentity {
    name: String,
    description: String,
    version: String,
}

fn fallback_identity(kind: AssetKind, format: &str, content: &str, id: &str) -> AssetIdentity {
    let (name, description, candidate_version) = match kind {
        AssetKind::Skill => {
            let value = content
                .splitn(3, "---")
                .nth(1)
                .and_then(|raw| serde_yaml::from_str::<serde_yaml::Value>(raw).ok());
            yaml_identity_hint(value.as_ref(), id)
        }
        AssetKind::Workflow if format == "toml" => {
            let value = toml::from_str::<toml::Value>(content).ok();
            toml_identity_hint(value.as_ref(), id)
        }
        AssetKind::Workflow => {
            let value = serde_yaml::from_str::<serde_yaml::Value>(content).ok();
            yaml_identity_hint(value.as_ref(), id)
        }
        AssetKind::Persona => {
            let value = toml::from_str::<toml::Value>(content).ok();
            toml_identity_hint(value.as_ref(), id)
        }
    };
    let version = candidate_version
        .filter(|value| validate_semver(value).is_ok())
        .unwrap_or_else(|| "1.0.0".to_string());
    AssetIdentity {
        name,
        description,
        version,
    }
}

fn yaml_identity_hint(
    value: Option<&serde_yaml::Value>,
    fallback: &str,
) -> (String, String, Option<String>) {
    let name = value
        .and_then(|value| value.get("name"))
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or(fallback)
        .to_string();
    let description = value
        .and_then(|value| value.get("description"))
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let version = value
        .and_then(|value| value.get("version"))
        .and_then(serde_yaml::Value::as_str)
        .map(ToOwned::to_owned);
    (name, description, version)
}

fn toml_identity_hint(
    value: Option<&toml::Value>,
    fallback: &str,
) -> (String, String, Option<String>) {
    let name = value
        .and_then(|value| value.get("name"))
        .and_then(toml::Value::as_str)
        .unwrap_or(fallback)
        .to_string();
    let description = value
        .and_then(|value| value.get("description"))
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let version = value
        .and_then(|value| value.get("version"))
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned);
    (name, description, version)
}

fn validate_content(
    kind: AssetKind,
    format: &str,
    content: &str,
) -> Result<AssetIdentity, AssetError> {
    validate_size(content)?;
    match kind {
        AssetKind::Skill => validate_skill(content),
        AssetKind::Workflow => validate_workflow(format, content),
        AssetKind::Persona => validate_persona(content),
    }
}

fn validate_skill(content: &str) -> Result<AssetIdentity, AssetError> {
    let parts = content.splitn(3, "---").collect::<Vec<_>>();
    if parts.len() < 3 || !parts[0].trim().is_empty() {
        return Err(AssetError::Invalid(vec![
            "skill must start with YAML frontmatter delimited by ---".to_string(),
        ]));
    }
    let frontmatter = serde_yaml::from_str::<SkillFrontmatter>(parts[1]).map_err(|error| {
        AssetError::Invalid(vec![format!("invalid skill frontmatter: {error}")])
    })?;
    let mut errors = Vec::new();
    if frontmatter.name.trim().is_empty() {
        errors.push("skill name must not be empty".to_string());
    }
    if frontmatter.description.trim().is_empty() {
        errors.push("skill description must not be empty".to_string());
    }
    if !frontmatter.trigger.starts_with('/') || frontmatter.trigger.len() < 2 {
        errors.push("skill trigger must start with '/'".to_string());
    }
    if parts[2].trim().is_empty() {
        errors.push("skill body must not be empty".to_string());
    }
    if let Err(error) = validate_semver(&frontmatter.version) {
        errors.push(error.to_string());
    }
    if !errors.is_empty() {
        return Err(AssetError::Invalid(errors));
    }
    Ok(AssetIdentity {
        name: frontmatter.name,
        description: frontmatter.description,
        version: frontmatter.version,
    })
}

fn validate_workflow(format: &str, content: &str) -> Result<AssetIdentity, AssetError> {
    let definition = match format {
        "toml" => toml::from_str::<WorkflowDef>(content).map_err(|error| {
            AssetError::Invalid(vec![format!("invalid workflow TOML: {error}")])
        })?,
        "yaml" | "yml" => serde_yaml::from_str::<WorkflowDef>(content).map_err(|error| {
            AssetError::Invalid(vec![format!("invalid workflow YAML: {error}")])
        })?,
        _ => {
            return Err(AssetError::Invalid(vec![format!(
                "unsupported workflow format '{format}'"
            )]))
        }
    };
    definition
        .validate_schema()
        .map_err(|error| AssetError::Invalid(vec![error.to_string()]))?;
    DagValidator::new(&definition)
        .validate()
        .map_err(|error| AssetError::Invalid(vec![error.to_string()]))?;
    let version = extract_version(format, content).unwrap_or_else(|| "1.0.0".to_string());
    validate_semver(&version)?;
    Ok(AssetIdentity {
        name: definition.name,
        description: definition.description.unwrap_or_default(),
        version,
    })
}

fn validate_persona(content: &str) -> Result<AssetIdentity, AssetError> {
    let persona = toml::from_str::<PersonaSpec>(content)
        .map_err(|error| AssetError::Invalid(vec![format!("invalid persona TOML: {error}")]))?;
    let mut errors = Vec::new();
    if persona.name.trim().is_empty() {
        errors.push("persona name must not be empty".to_string());
    }
    if persona.description.trim().is_empty() {
        errors.push("persona description must not be empty".to_string());
    }
    if persona.system_prompt.trim().is_empty() {
        errors.push("persona system_prompt must not be empty".to_string());
    }
    if persona.preferred_model.trim().is_empty() {
        errors.push("persona preferred_model must not be empty".to_string());
    }
    let version = extract_version("toml", content).unwrap_or_else(|| "1.0.0".to_string());
    if let Err(error) = validate_semver(&version) {
        errors.push(error.to_string());
    }
    if !errors.is_empty() {
        return Err(AssetError::Invalid(errors));
    }
    Ok(AssetIdentity {
        name: persona.name,
        description: persona.description,
        version,
    })
}

fn normalize_format(kind: AssetKind, format: Option<&str>) -> Result<String, AssetError> {
    let format = format
        .unwrap_or(kind.default_extension())
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if kind.supports_extension(&format) {
        Ok(format)
    } else {
        Err(AssetError::BadRequest(format!(
            "format '{format}' is not supported for {kind} assets"
        )))
    }
}

fn validate_id(id: &str) -> Result<(), AssetError> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(AssetError::BadRequest(
            "asset id must contain only ASCII letters, numbers, '-' or '_'".to_string(),
        ));
    }
    Ok(())
}

fn validate_revision(revision: &str) -> Result<(), AssetError> {
    if revision.len() != 64 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AssetError::BadRequest(
            "expected_revision must be a SHA-256 revision".to_string(),
        ));
    }
    Ok(())
}

fn validate_size(content: &str) -> Result<(), AssetError> {
    if content.len() > MAX_ASSET_BYTES {
        return Err(AssetError::TooLarge(format!(
            "asset exceeds the {} byte limit",
            MAX_ASSET_BYTES
        )));
    }
    Ok(())
}

fn ensure_safe_directory(path: &Path) -> Result<(), AssetError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| io_error("inspect global asset directory", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AssetError::Forbidden(
            "global asset directory must be a real directory, not a symlink".to_string(),
        ));
    }
    Ok(())
}

fn ensure_writable_directory(path: &Path) -> Result<(), AssetError> {
    if path.exists() {
        return ensure_safe_directory(path);
    }
    fs::create_dir_all(path).map_err(|error| io_error("create global asset directory", error))?;
    ensure_safe_directory(path)
}

fn ensure_safe_file(path: &Path) -> Result<(), AssetError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| io_error("inspect global asset", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AssetError::Forbidden(
            "global assets must be regular files, not symlinks".to_string(),
        ));
    }
    Ok(())
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), AssetError> {
    let parent = path
        .parent()
        .ok_or_else(|| AssetError::Forbidden("asset path has no parent directory".to_string()))?;
    ensure_safe_directory(parent)?;
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AssetError::Forbidden("asset filename is not valid UTF-8".to_string()))?;
    let temporary = parent.join(format!(".{filename}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = File::options()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| io_error("create temporary asset file", error))?;
        file.write_all(content)
            .map_err(|error| io_error("write temporary asset file", error))?;
        file.sync_all()
            .map_err(|error| io_error("sync temporary asset file", error))?;

        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)
                .map_err(|error| io_error("remove existing global asset", error))?;
        }

        fs::rename(&temporary, path).map_err(|error| io_error("replace global asset", error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn revision(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn bump_version(version: &str, bump: VersionBump) -> Result<String, AssetError> {
    let [major, minor, patch] = validate_semver(version)?;
    let next = match bump {
        VersionBump::None => [major, minor, patch],
        VersionBump::Patch => [major, minor, patch.saturating_add(1)],
        VersionBump::Minor => [major, minor.saturating_add(1), 0],
        VersionBump::Major => [major.saturating_add(1), 0, 0],
    };
    Ok(format!("{}.{}.{}", next[0], next[1], next[2]))
}

fn validate_semver(version: &str) -> Result<[u64; 3], AssetError> {
    let numbers = version
        .trim()
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AssetError::Invalid(vec![format!("version '{version}' must be x.y.z")]))?;
    if numbers.len() != 3 {
        return Err(AssetError::Invalid(vec![format!(
            "version '{version}' must be x.y.z"
        )]));
    }
    Ok([numbers[0], numbers[1], numbers[2]])
}

fn set_version(
    kind: AssetKind,
    format: &str,
    content: &str,
    version: &str,
) -> Result<String, AssetError> {
    validate_semver(version)?;
    match kind {
        AssetKind::Skill => set_skill_version(content, version),
        AssetKind::Workflow | AssetKind::Persona => {
            set_structured_version(format, content, version)
        }
    }
}

fn set_skill_version(content: &str, version: &str) -> Result<String, AssetError> {
    let mut lines = content.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    if lines.first().map(|line| line.trim()) != Some("---") {
        return Err(AssetError::Invalid(vec![
            "skill must start with YAML frontmatter delimited by ---".to_string(),
        ]));
    }
    let end = lines
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, line)| (line.trim() == "---").then_some(index))
        .ok_or_else(|| AssetError::Invalid(vec!["skill frontmatter is not closed".to_string()]))?;
    if let Some(index) = (1..end).find(|index| lines[*index].trim_start().starts_with("version:")) {
        lines[index] = format!("version: \"{version}\"");
    } else {
        lines.insert(end, format!("version: \"{version}\""));
    }
    let mut output = lines.join("\n");
    if content.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

fn set_structured_version(
    format: &str,
    content: &str,
    version: &str,
) -> Result<String, AssetError> {
    let mut lines = content.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let marker = if format == "toml" {
        "version"
    } else {
        "version:"
    };
    let replacement = if format == "toml" {
        format!("version = \"{version}\"")
    } else {
        format!("version: \"{version}\"")
    };
    let existing = lines.iter().position(|line| {
        let trimmed = line.trim_start();
        !line.starts_with(char::is_whitespace)
            && if format == "toml" {
                trimmed.starts_with(marker) && trimmed[marker.len()..].trim_start().starts_with('=')
            } else {
                trimmed.starts_with(marker)
            }
    });
    if let Some(index) = existing {
        lines[index] = replacement;
    } else {
        let insertion = lines
            .iter()
            .position(|line| {
                let trimmed = line.trim_start();
                !line.starts_with(char::is_whitespace)
                    && if format == "toml" {
                        trimmed.starts_with("name") && trimmed[4..].trim_start().starts_with('=')
                    } else {
                        trimmed.starts_with("name:")
                    }
            })
            .map(|index| index + 1)
            .unwrap_or(0);
        lines.insert(insertion, replacement);
    }
    let mut output = lines.join("\n");
    if content.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

fn extract_version(format: &str, content: &str) -> Option<String> {
    if format == "toml" {
        return toml::from_str::<toml::Value>(content)
            .ok()?
            .get("version")?
            .as_str()
            .map(ToOwned::to_owned);
    }
    serde_yaml::from_str::<serde_yaml::Value>(content)
        .ok()?
        .get("version")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn io_error(action: &str, error: std::io::Error) -> AssetError {
    AssetError::Io(format!("failed to {action}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILL: &str = r#"---
name: Demo
description: Demo skill
trigger: /demo
version: "1.2.3"
---

Do the demo work.
"#;

    const WORKFLOW: &str = r#"name: demo
description: Demo workflow
steps:
  - id: do_it
    agent: Coder
    prompt: Do the work
    output: result
"#;

    const PERSONA: &str = r#"name = "Demo"
description = "Demo persona"
preferred_model = "codex"
allowed_tools = []
system_prompt = "Do excellent work."
"#;

    #[test]
    fn validates_all_three_global_asset_kinds() {
        let store = GlobalAssetStore::new("unused");
        assert!(store.validate(AssetKind::Skill, Some("md"), SKILL).valid);
        assert!(
            store
                .validate(AssetKind::Workflow, Some("yaml"), WORKFLOW)
                .valid
        );
        assert!(
            store
                .validate(AssetKind::Persona, Some("toml"), PERSONA)
                .valid
        );
    }

    #[test]
    fn create_update_and_delete_are_versioned_and_recoverable() {
        let temp = tempfile::tempdir().unwrap();
        let store = GlobalAssetStore::new(temp.path());
        let created = store
            .create(AssetKind::Skill, "demo", Some("md"), SKILL)
            .unwrap();
        assert_eq!(created.summary.version, "1.0.0");

        let updated = store
            .update(
                AssetKind::Skill,
                "demo",
                &created.summary.revision,
                VersionBump::Minor,
                &created.content.replace("Demo skill", "Improved skill"),
            )
            .unwrap();
        assert_eq!(updated.summary.version, "1.1.0");
        assert!(temp
            .path()
            .join("backups/hub/skills")
            .read_dir()
            .unwrap()
            .next()
            .is_some());

        let stale = store.update(
            AssetKind::Skill,
            "demo",
            &created.summary.revision,
            VersionBump::Patch,
            &updated.content,
        );
        assert!(matches!(stale, Err(AssetError::Conflict(_))));

        store
            .delete(AssetKind::Skill, "demo", &updated.summary.revision, "demo")
            .unwrap();
        assert!(matches!(
            store.get(AssetKind::Skill, "demo"),
            Err(AssetError::NotFound(_))
        ));
        assert!(temp
            .path()
            .join("trash/hub/skills")
            .read_dir()
            .unwrap()
            .next()
            .is_some());
    }

    #[test]
    fn rejects_traversal_and_symlink_assets() {
        let temp = tempfile::tempdir().unwrap();
        let store = GlobalAssetStore::new(temp.path());
        assert!(matches!(
            store.create(AssetKind::Skill, "../escape", Some("md"), SKILL),
            Err(AssetError::BadRequest(_))
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let skills = temp.path().join("skills");
            fs::create_dir_all(&skills).unwrap();
            let target = temp.path().join("target.md");
            fs::write(&target, SKILL).unwrap();
            symlink(&target, skills.join("linked.md")).unwrap();
            assert!(matches!(
                store.get(AssetKind::Skill, "linked"),
                Err(AssetError::NotFound(_))
            ));
        }
    }

    #[test]
    fn version_bumps_cover_none_patch_minor_and_major() {
        assert_eq!(bump_version("1.2.3", VersionBump::None).unwrap(), "1.2.3");
        assert_eq!(bump_version("1.2.3", VersionBump::Patch).unwrap(), "1.2.4");
        assert_eq!(bump_version("1.2.3", VersionBump::Minor).unwrap(), "1.3.0");
        assert_eq!(bump_version("1.2.3", VersionBump::Major).unwrap(), "2.0.0");
    }

    #[test]
    fn invalid_asset_remains_visible_and_openable_for_repair() {
        let temp = tempfile::tempdir().unwrap();
        let skills = temp.path().join("skills");
        fs::create_dir_all(&skills).unwrap();
        fs::write(
            skills.join("broken.md"),
            "---\nname: Broken\ndescription: Needs repair\ntrigger: invalid\n---\n",
        )
        .unwrap();
        let store = GlobalAssetStore::new(temp.path());

        let listed = store.list(AssetKind::Skill).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].valid);
        assert!(!listed[0].validation_errors.is_empty());
        let opened = store.get(AssetKind::Skill, "broken").unwrap();
        assert_eq!(opened.summary.name, "Broken");
        assert!(opened.content.contains("trigger: invalid"));
    }
}
