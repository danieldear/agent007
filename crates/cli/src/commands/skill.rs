use crate::config::Config;
use crate::SkillAction;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub trigger: String,
    pub version: String,
}

/// Copy a skill file into the skills directory (preserving filename).
pub fn copy_skill_to_dir(skill_path: &Path, skills_dir: &Path) -> Result<()> {
    let filename = skill_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid skill path"))?;
    let dest = skills_dir.join(filename);
    std::fs::create_dir_all(skills_dir)?;
    std::fs::copy(skill_path, dest)?;
    Ok(())
}

/// Minimal frontmatter struct for listing — model is optional to allow skill files without it.
#[derive(serde::Deserialize)]
struct ListFrontmatter {
    name: String,
    description: String,
    trigger: String,
    #[allow(dead_code)]
    model: Option<String>,
    #[serde(default = "default_version")]
    version: String,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

/// List all skills found in skills_dir. Returns a Vec of summaries (name + description + trigger).
/// Reads each .md file and parses YAML frontmatter.
pub async fn list_skills(skills_dir: &Path) -> Result<Vec<SkillSummary>> {
    let mut summaries = Vec::new();

    let entries = std::fs::read_dir(skills_dir)
        .map_err(|e| anyhow::anyhow!("cannot read skills dir {}: {}", skills_dir.display(), e))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        // Only process .md files
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        let content = std::fs::read_to_string(&path)?;

        // Split on "---" to extract frontmatter
        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            // skip files without valid frontmatter
            continue;
        }

        let fm: ListFrontmatter = serde_yaml::from_str(parts[1])
            .map_err(|e| anyhow::anyhow!("frontmatter parse error in {}: {}", path.display(), e))?;

        summaries.push(SkillSummary {
            name: fm.name,
            description: fm.description,
            trigger: fm.trigger,
            version: fm.version,
        });
    }

    Ok(summaries)
}

/// Execute a skill by trigger string with provided args string.
pub async fn run_skill(
    trigger: &str,
    args: &str,
    executor: &agent007_skills::SkillExecutor,
) -> Result<String> {
    // Load skills from the default directory and find matching trigger
    let skills_dir = default_skills_dir();
    let loader = agent007_skills::SkillLoader::new(&skills_dir);
    let skills = loader
        .load_all()
        .map_err(|e| anyhow::anyhow!("failed to load skills: {}", e))?;

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
            let summaries = list_skills(&skills_dir).await?;
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
            let is_dry_run = crate::commands::run::is_dry_run();
            let router = Arc::new(crate::commands::run::build_model_router(
                &config, is_dry_run,
            ));

            // Embedding provider + VectorDB for the Retriever
            let embedder = Arc::new(agent007_models::MockProvider::with_embedding_dim(
                "",
                "mock-embed",
                384,
            )) as Arc<dyn agent007_models::EmbeddingProvider>;

            let db: Arc<dyn agent007_memory::VectorDB> = if is_dry_run {
                Arc::new(crate::commands::run::NoOpVectorDB)
            } else {
                let home = crate::commands::run::agent007_home();
                let vdb_path = home.join("vectordb");
                std::fs::create_dir_all(&vdb_path)?;
                let vdb_path_str = vdb_path.to_string_lossy().to_string();
                let store =
                    agent007_memory::vectordb::LanceDBStore::new(&vdb_path_str, "skills", 384)
                        .await
                        .map_err(|e| anyhow::anyhow!("failed to open vector db: {}", e))?;
                Arc::new(store)
            };

            let retriever = Arc::new(agent007_memory::Retriever::new(embedder, db, 5));

            let home = crate::commands::run::agent007_home();
            let memory_dir = home.join("memory");
            let memory_store = Arc::new(agent007_memory::store::MemoryStore::new(memory_dir));
            let memory = memory_store.global();
            let global_store = Arc::new(agent007_memory::store::MemoryStore::new(
                crate::commands::run::agent007_global_home().join("memory"),
            ));
            let global_memory = global_store.scoped("global");

            let executor = agent007_skills::SkillExecutor::new(
                router as Arc<dyn agent007_models::ModelProvider>,
                retriever,
                memory,
            )
            .with_global_memory(global_memory);

            let result = run_skill(&trigger, &args, &executor).await?;
            println!("{}", result);
            Ok(())
        }
        SkillAction::Install { source } => install_skill(&source, &skills_dir),
    }
}

fn default_skills_dir() -> PathBuf {
    crate::commands::run::agent007_home().join("skills")
}

/// Install a skill from a GitHub path or HTTPS URL.
///
/// Supported source formats:
/// - `github:owner/repo/path/to/skill.md` → fetches from raw.githubusercontent.com/owner/repo/HEAD/path
/// - `https://...` → fetches directly
pub fn install_skill(source: &str, skills_dir: &std::path::Path) -> Result<()> {
    let url = if let Some(gh) = source.strip_prefix("github:") {
        // github:owner/repo/path/to/skill.md
        let parts: Vec<&str> = gh.splitn(3, '/').collect();
        if parts.len() < 3 {
            anyhow::bail!("invalid github source — expected github:owner/repo/path/to/skill.md");
        }
        let (owner, repo, path) = (parts[0], parts[1], parts[2]);
        format!("https://raw.githubusercontent.com/{owner}/{repo}/HEAD/{path}")
    } else if source.starts_with("https://") || source.starts_with("http://") {
        source.to_string()
    } else {
        anyhow::bail!("unsupported source — use github:owner/repo/path.md or https://...");
    };

    let response = reqwest::blocking::get(&url)
        .map_err(|e| anyhow::anyhow!("failed to fetch skill from {url}: {e}"))?;

    if !response.status().is_success() {
        anyhow::bail!("fetch failed — HTTP {} for {url}", response.status());
    }

    let content = response
        .text()
        .map_err(|e| anyhow::anyhow!("failed to read response body: {e}"))?;

    // Validate frontmatter: must have name: and trigger:
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        anyhow::bail!("skill file has no YAML frontmatter (expected --- delimiters)");
    }

    #[derive(serde::Deserialize)]
    struct MinimalFrontmatter {
        name: String,
        trigger: String,
    }
    let fm: MinimalFrontmatter = serde_yaml::from_str(parts[1])
        .map_err(|e| anyhow::anyhow!("failed to parse skill frontmatter: {e}"))?;

    // Derive filename from trigger (strip leading /)
    let filename = format!(
        "{}.md",
        fm.trigger.trim_start_matches('/').replace('/', "-")
    );
    std::fs::create_dir_all(skills_dir)?;
    let dest = skills_dir.join(&filename);
    std::fs::write(&dest, &content)?;

    println!("Installed skill '{}' → {}", fm.name, dest.display());
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
}
