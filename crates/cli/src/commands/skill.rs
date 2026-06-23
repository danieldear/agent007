use crate::config::Config;
use crate::SkillAction;
use anyhow::Result;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub trigger: String,
    pub version: String,
}

#[derive(Debug)]
enum InstallSource {
    DirectFile {
        url: String,
    },
    GitHubFile {
        owner: String,
        repo: String,
        reference: Option<String>,
        path: String,
    },
    GitHubDir {
        owner: String,
        repo: String,
        reference: Option<String>,
        path: String,
    },
}

#[derive(Debug, Deserialize)]
struct MinimalFrontmatter {
    name: String,
    trigger: String,
}

#[derive(Debug, Deserialize)]
struct GitHubContentEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    download_url: Option<String>,
}

/// Copy a skill file or package directory into the skills directory.
pub fn copy_skill_to_dir(skill_path: &Path, skills_dir: &Path) -> Result<()> {
    fs::create_dir_all(skills_dir)?;
    if skill_path.is_dir() {
        let manifest = skill_path.join("SKILL.md");
        if !manifest.is_file() {
            anyhow::bail!(
                "skill directory {} is missing SKILL.md",
                skill_path.display()
            );
        }
        let dirname = skill_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("invalid skill path"))?;
        let dest = skills_dir.join(dirname);
        copy_dir_recursive(skill_path, &dest)?;
        return Ok(());
    }

    let filename = skill_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid skill path"))?;
    let dest = skills_dir.join(filename);
    fs::copy(skill_path, dest)?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let entry_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if entry_path.is_dir() {
            copy_dir_recursive(&entry_path, &dest_path)?;
        } else {
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&entry_path, &dest_path)?;
        }
    }
    Ok(())
}

/// List all skills found in skills_dir. Returns a Vec of summaries (name + description + trigger).
/// Reads each .md file and parses YAML frontmatter.
pub async fn list_skills(skills_dir: &Path) -> Result<Vec<SkillSummary>> {
    let loader = agent007_skills::SkillLoader::new(skills_dir);
    let skills = loader
        .load_all()
        .map_err(|e| anyhow::anyhow!("failed to load skills: {}", e))?;

    Ok(skills
        .into_iter()
        .map(|skill| SkillSummary {
            name: skill.name().to_string(),
            description: skill.frontmatter.description.clone(),
            trigger: skill.trigger().to_string(),
            version: skill.version().to_string(),
        })
        .collect())
}

/// Execute a skill by trigger string with provided args string.
pub async fn run_skill(
    trigger: &str,
    args: &str,
    executor: &agent007_skills::SkillExecutor,
) -> Result<String> {
    // Load skills with project-over-global precedence and find matching trigger
    let skills = load_skills_with_precedence()?;

    let skill = skills
        .into_iter()
        .find(|s| s.trigger() == trigger)
        .ok_or_else(|| anyhow::anyhow!("no skill found with trigger: {}", trigger))?;

    let result = executor
        .execute(&skill, args)
        .await
        .map_err(|e| anyhow::anyhow!("skill execution failed: {}", e))?;

    Ok(result)
}

/// Top-level dispatch for `agent007 skill <action>`.
pub async fn execute(config: Arc<Config>, action: SkillAction) -> Result<()> {
    let skills_dir = default_skills_dir();
    match action {
        SkillAction::List => {
            let summaries: Vec<SkillSummary> = load_skills_with_precedence()?
                .into_iter()
                .map(|skill| SkillSummary {
                    name: skill.name().to_string(),
                    description: skill.frontmatter.description.clone(),
                    trigger: skill.trigger().to_string(),
                    version: skill.version().to_string(),
                })
                .collect();
            for s in &summaries {
                println!(
                    "[v{}] {:20} {:40} {}",
                    s.version, s.trigger, s.name, s.description
                );
            }
            Ok(())
        }
        SkillAction::Add { path } => copy_skill_to_dir(std::path::Path::new(&path), &skills_dir),
        SkillAction::Run { trigger, args } => {
            // Reuse the same stack/executor path as `agent007 run` so skill runs
            // benefit from the exact same retrieval, memory, and routing behavior.
            let stack = crate::commands::run::build_stack(config.as_ref()).await?;
            let result = run_skill(&trigger, &args, &stack.skill_executor).await?;
            println!("{}", result);
            Ok(())
        }
        SkillAction::Install { source } => install_skill(&source, &skills_dir),
    }
}

fn default_skills_dir() -> PathBuf {
    crate::commands::run::agent007_write_home().join("skills")
}

fn configured_skill_dirs() -> Vec<PathBuf> {
    agent007_core::paths::skills_search_dirs()
}

fn load_skills_with_precedence() -> Result<Vec<agent007_skills::Skill>> {
    let mut skills: BTreeMap<String, agent007_skills::Skill> = BTreeMap::new();
    for dir in configured_skill_dirs() {
        if !dir.exists() {
            continue;
        }
        let loader = agent007_skills::SkillLoader::new(&dir);
        for skill in loader
            .load_all()
            .map_err(|e| anyhow::anyhow!("failed to load skills from {}: {}", dir.display(), e))?
        {
            skills.entry(skill.trigger().to_string()).or_insert(skill);
        }
    }
    Ok(skills.into_values().collect())
}

/// Install a skill from a GitHub path or HTTPS URL.
///
/// Supported source formats:
/// - `github:owner/repo/path/to/skill.md` → fetches from raw.githubusercontent.com/owner/repo/HEAD/path
/// - `https://...` → fetches directly
pub fn install_skill(source: &str, skills_dir: &std::path::Path) -> Result<()> {
    fs::create_dir_all(skills_dir)?;
    let source = parse_install_source(source)?;
    let client = github_client()?;

    match source {
        InstallSource::DirectFile { url } => {
            let content = fetch_text(&client, &url)?;
            let fm = parse_minimal_frontmatter(&content)?;
            let filename = format!(
                "{}.md",
                fm.trigger.trim_start_matches('/').replace('/', "-")
            );
            let dest = skills_dir.join(&filename);
            fs::write(&dest, &content)?;
            println!("Installed skill '{}' → {}", fm.name, dest.display());
            Ok(())
        }
        InstallSource::GitHubFile {
            owner,
            repo,
            reference,
            path,
        } => {
            if Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case("SKILL.md"))
                .unwrap_or(false)
            {
                let package_path = Path::new(&path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                let package = fetch_github_package(
                    &client,
                    &owner,
                    &repo,
                    reference.as_deref(),
                    &package_path,
                )?;
                write_skill_package(skills_dir, &package)?;
                println!(
                    "Installed skill package '{}' → {}",
                    package.frontmatter.name,
                    skills_dir.join(&package.package_name).display()
                );
                return Ok(());
            }

            let url = github_raw_file_url(&owner, &repo, reference.as_deref(), &path);
            let content = fetch_text(&client, &url)?;
            let fm = parse_minimal_frontmatter(&content)?;
            let filename = format!(
                "{}.md",
                fm.trigger.trim_start_matches('/').replace('/', "-")
            );
            let dest = skills_dir.join(&filename);
            fs::write(&dest, &content)?;
            println!("Installed skill '{}' → {}", fm.name, dest.display());
            Ok(())
        }
        InstallSource::GitHubDir {
            owner,
            repo,
            reference,
            path,
        } => {
            let package =
                fetch_github_package(&client, &owner, &repo, reference.as_deref(), &path)?;
            write_skill_package(skills_dir, &package)?;
            println!(
                "Installed skill package '{}' → {}",
                package.frontmatter.name,
                skills_dir.join(&package.package_name).display()
            );
            Ok(())
        }
    }
}

#[derive(Debug)]
struct SkillPackageContent {
    frontmatter: MinimalFrontmatter,
    package_name: String,
    files: Vec<(String, String)>,
}

fn parse_install_source(source: &str) -> Result<InstallSource> {
    let source = source.trim();
    if let Some(gh) = source.strip_prefix("github:") {
        let parts: Vec<&str> = gh.split('/').collect();
        if parts.len() < 2 {
            anyhow::bail!(
                "invalid github source — expected github:owner/repo[/path/to/skill(.md)]"
            );
        }
        let owner = parts[0].to_string();
        let repo = parts[1].to_string();
        let path = if parts.len() > 2 {
            parts[2..].join("/")
        } else {
            String::new()
        };
        return Ok(if path.ends_with(".md") {
            InstallSource::GitHubFile {
                owner,
                repo,
                reference: Some("HEAD".to_string()),
                path,
            }
        } else {
            InstallSource::GitHubDir {
                owner,
                repo,
                reference: Some("HEAD".to_string()),
                path,
            }
        });
    }

    if source.starts_with("https://raw.githubusercontent.com/")
        || source.starts_with("http://raw.githubusercontent.com/")
    {
        let normalized = source
            .trim_start_matches("https://raw.githubusercontent.com/")
            .trim_start_matches("http://raw.githubusercontent.com/");
        let parts: Vec<&str> = normalized.split('/').collect();
        if parts.len() >= 4 {
            let owner = parts[0].to_string();
            let repo = parts[1].to_string();
            let reference = Some(parts[2].to_string());
            let path = parts[3..].join("/");
            return Ok(InstallSource::GitHubFile {
                owner,
                repo,
                reference,
                path,
            });
        }
    }

    if source.contains("github.com") {
        let without_scheme = source
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let parts: Vec<&str> = without_scheme.split('/').collect();
        if parts.len() >= 3 && parts[0] == "github.com" {
            let owner = parts[1].to_string();
            let repo = parts[2].to_string();
            if parts.len() >= 5 && parts[3] == "blob" {
                let reference = Some(parts[4].to_string());
                let path = parts[5..].join("/");
                return Ok(InstallSource::GitHubFile {
                    owner,
                    repo,
                    reference,
                    path,
                });
            }
            if parts.len() >= 5 && parts[3] == "tree" {
                let reference = Some(parts[4].to_string());
                let path = parts[5..].join("/");
                return Ok(InstallSource::GitHubDir {
                    owner,
                    repo,
                    reference,
                    path,
                });
            }
            return Ok(InstallSource::GitHubDir {
                owner,
                repo,
                reference: None,
                path: String::new(),
            });
        }
    }

    if source.starts_with("https://") || source.starts_with("http://") {
        return Ok(InstallSource::DirectFile {
            url: source.to_string(),
        });
    }

    anyhow::bail!(
        "unsupported source — use github:owner/repo[/path], GitHub tree/blob URL, or https://..."
    );
}

fn github_client() -> Result<Client> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("agent007"));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))
}

fn fetch_text(client: &Client, url: &str) -> Result<String> {
    let response = client
        .get(url)
        .send()
        .map_err(|e| anyhow::anyhow!("failed to fetch skill from {url}: {e}"))?;
    if !response.status().is_success() {
        anyhow::bail!("fetch failed — HTTP {} for {url}", response.status());
    }
    response
        .text()
        .map_err(|e| anyhow::anyhow!("failed to read response body: {e}"))
}

fn parse_minimal_frontmatter(content: &str) -> Result<MinimalFrontmatter> {
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        anyhow::bail!("skill file has no YAML frontmatter (expected --- delimiters)");
    }
    serde_yaml::from_str(parts[1])
        .map_err(|e| anyhow::anyhow!("failed to parse skill frontmatter: {e}"))
}

fn sanitize_skill_slug(value: &str, fallback: &str) -> String {
    let slug: String = value
        .trim_start_matches('/')
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug.to_string()
    }
}

fn github_raw_file_url(owner: &str, repo: &str, reference: Option<&str>, path: &str) -> String {
    let reference = reference.unwrap_or("HEAD");
    format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/{reference}/{}",
        path.trim_start_matches('/')
    )
}

fn github_contents_api_url(owner: &str, repo: &str, reference: Option<&str>, path: &str) -> String {
    let mut url = if path.trim().is_empty() {
        format!("https://api.github.com/repos/{owner}/{repo}/contents")
    } else {
        format!(
            "https://api.github.com/repos/{owner}/{repo}/contents/{}",
            path.trim_start_matches('/')
        )
    };
    if let Some(reference) = reference {
        url.push_str(&format!("?ref={reference}"));
    }
    url
}

fn fetch_github_package(
    client: &Client,
    owner: &str,
    repo: &str,
    reference: Option<&str>,
    path: &str,
) -> Result<SkillPackageContent> {
    let mut files = Vec::new();
    fetch_github_package_recursive(client, owner, repo, reference, path, path, &mut files)?;

    let skill_entry = files
        .iter()
        .find(|(relative, _)| relative.eq_ignore_ascii_case("SKILL.md"))
        .ok_or_else(|| anyhow::anyhow!("package is missing SKILL.md"))?;
    let frontmatter = parse_minimal_frontmatter(&skill_entry.1)?;

    let fallback_name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("imported-skill");
    let package_name = sanitize_skill_slug(&frontmatter.trigger, fallback_name);

    Ok(SkillPackageContent {
        frontmatter,
        package_name,
        files,
    })
}

fn fetch_github_package_recursive(
    client: &Client,
    owner: &str,
    repo: &str,
    reference: Option<&str>,
    package_root: &str,
    current_path: &str,
    files: &mut Vec<(String, String)>,
) -> Result<()> {
    let api_url = github_contents_api_url(owner, repo, reference, current_path);
    let response = client
        .get(&api_url)
        .send()
        .map_err(|e| anyhow::anyhow!("failed to fetch package listing from {api_url}: {e}"))?;
    if !response.status().is_success() {
        anyhow::bail!(
            "package listing fetch failed — HTTP {} for {api_url}",
            response.status()
        );
    }

    let entries: Vec<GitHubContentEntry> = response
        .json()
        .map_err(|e| anyhow::anyhow!("failed to parse GitHub package listing: {e}"))?;

    for entry in entries {
        match entry.kind.as_str() {
            "file" => {
                let download_url = entry.download_url.ok_or_else(|| {
                    anyhow::anyhow!("GitHub did not return a download URL for {}", entry.path)
                })?;
                let content = fetch_text(client, &download_url)?;
                let relative = Path::new(&entry.path)
                    .strip_prefix(Path::new(package_root))
                    .map_err(|_| {
                        anyhow::anyhow!("failed to derive relative path for {}", entry.path)
                    })?
                    .to_string_lossy()
                    .to_string();
                files.push((relative, content));
            }
            "dir" => {
                fetch_github_package_recursive(
                    client,
                    owner,
                    repo,
                    reference,
                    package_root,
                    &entry.path,
                    files,
                )?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn write_skill_package(skills_dir: &Path, package: &SkillPackageContent) -> Result<()> {
    let package_dir = skills_dir.join(&package.package_name);
    fs::create_dir_all(&package_dir)?;
    for (relative, content) in &package.files {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            anyhow::bail!("invalid package entry path: {}", relative);
        }
        let dest = package_dir.join(relative_path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(dest, content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn skill_add_copies_file_to_skills_dir() {
        let skills_dir = TempDir::new().unwrap();
        let src_dir = TempDir::new().unwrap();
        let skill_file = src_dir.path().join("my_skill.md");
        // write a valid skill file with frontmatter
        std::fs::write(
            &skill_file,
            "---\nname: my_skill\ndescription: test skill\ntrigger: /my-skill\n---\nDo something.\n",
        )
        .unwrap();
        copy_skill_to_dir(&skill_file, skills_dir.path()).unwrap();
        assert!(skills_dir.path().join("my_skill.md").exists());
    }

    #[test]
    fn skill_add_copies_directory_package_to_skills_dir() {
        let skills_dir = TempDir::new().unwrap();
        let src_dir = TempDir::new().unwrap();
        let package_dir = src_dir.path().join("review-skill");
        std::fs::create_dir_all(package_dir.join("assets")).unwrap();
        std::fs::write(
            package_dir.join("SKILL.md"),
            "---\nname: review\ndescription: packaged skill\ntrigger: /review\nmodel: claude\n---\nUse {{skill_dir}}.\n",
        )
        .unwrap();
        std::fs::write(package_dir.join("assets").join("example.txt"), "example").unwrap();

        copy_skill_to_dir(&package_dir, skills_dir.path()).unwrap();

        assert!(skills_dir
            .path()
            .join("review-skill")
            .join("SKILL.md")
            .exists());
        assert!(skills_dir
            .path()
            .join("review-skill")
            .join("assets")
            .join("example.txt")
            .exists());
    }

    #[tokio::test]
    async fn skill_list_returns_loaded_skills() {
        let skills_dir = TempDir::new().unwrap();
        // write two skill files
        std::fs::write(
            skills_dir.path().join("skill_a.md"),
            "---\nname: skill_a\ndescription: first skill\ntrigger: /skill-a\n---\nDo A.\n",
        )
        .unwrap();
        std::fs::write(
            skills_dir.path().join("skill_b.md"),
            "---\nname: skill_b\ndescription: second skill\ntrigger: /skill-b\n---\nDo B.\n",
        )
        .unwrap();
        let summaries = list_skills(skills_dir.path()).await.unwrap();
        assert_eq!(summaries.len(), 2);
    }

    #[tokio::test]
    async fn skill_list_includes_version() {
        let skills_dir = TempDir::new().unwrap();
        std::fs::write(
            skills_dir.path().join("versioned.md"),
            "---\nname: versioned\ndescription: test\ntrigger: /versioned\nversion: \"2.1.0\"\n---\nDo it.\n",
        ).unwrap();
        let summaries = list_skills(skills_dir.path()).await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].version, "2.1.0");
    }

    #[tokio::test]
    async fn skill_list_defaults_version_when_missing() {
        let skills_dir = TempDir::new().unwrap();
        std::fs::write(
            skills_dir.path().join("no_version.md"),
            "---\nname: no_version\ndescription: test\ntrigger: /no-version\n---\nDo it.\n",
        )
        .unwrap();
        let summaries = list_skills(skills_dir.path()).await.unwrap();
        assert_eq!(summaries[0].version, "1.0.0");
    }

    #[test]
    fn install_skill_rejects_invalid_source() {
        let skills_dir = TempDir::new().unwrap();
        let err = install_skill("ftp://bad-scheme.example.com/skill.md", skills_dir.path());
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("unsupported source"));
    }

    #[test]
    fn install_skill_rejects_bad_github_path() {
        let skills_dir = TempDir::new().unwrap();
        let err = install_skill("github:only-one-segment", skills_dir.path());
        assert!(err.is_err());
    }

    #[test]
    fn parse_install_source_supports_github_tree_urls() {
        let parsed =
            parse_install_source("https://github.com/example/repo/tree/main/skills/review-skill")
                .unwrap();
        assert!(matches!(
            parsed,
            InstallSource::GitHubDir {
                owner,
                repo,
                reference: Some(reference),
                path
            } if owner == "example" && repo == "repo" && reference == "main" && path == "skills/review-skill"
        ));
    }

    #[test]
    fn parse_install_source_treats_skill_manifest_as_package_source() {
        let parsed = parse_install_source(
            "https://raw.githubusercontent.com/example/repo/main/skills/review-skill/SKILL.md",
        )
        .unwrap();
        assert!(matches!(
            parsed,
            InstallSource::GitHubFile {
                owner,
                repo,
                reference: Some(reference),
                path
            } if owner == "example" && repo == "repo" && reference == "main" && path == "skills/review-skill/SKILL.md"
        ));
    }
}
