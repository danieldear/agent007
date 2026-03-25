use std::path::PathBuf;
use crate::error::SkillError;
use crate::types::{Skill, SkillFrontmatter};

pub struct SkillLoader {
    skills_dir: PathBuf,
}

impl SkillLoader {
    pub fn new(skills_dir: impl Into<PathBuf>) -> Self {
        Self { skills_dir: skills_dir.into() }
    }

    pub fn load_all(&self) -> Result<Vec<Skill>, SkillError> {
        let mut skills = Vec::new();

        let entries = std::fs::read_dir(&self.skills_dir)
            .map_err(|e| SkillError::Io { path: self.skills_dir.clone(), source: e })?;

        for entry in entries {
            let entry = entry
                .map_err(|e| SkillError::Io { path: self.skills_dir.clone(), source: e })?;
            let path = entry.path();

            // Only process .md files
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            let content = std::fs::read_to_string(&path)
                .map_err(|e| SkillError::Io { path: path.clone(), source: e })?;

            // Split on "---" to get frontmatter
            // skill files start with "---\n" so splitn(3, "---") yields ["", " fm\n", " body\n"]
            let parts: Vec<&str> = content.splitn(3, "---").collect();
            if parts.len() < 3 {
                return Err(SkillError::MissingFrontmatter { path });
            }

            let frontmatter: SkillFrontmatter = serde_yaml::from_str(parts[1])
                .map_err(|e| SkillError::FrontmatterParse { path: path.clone(), source: e })?;

            let template = parts[2].trim().to_string();
            skills.push(Skill { frontmatter, template });
        }

        Ok(skills)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn loads_valid_skill_file() {
        let dir = TempDir::new().unwrap();
        let skill_path = dir.path().join("test.md");
        fs::write(&skill_path, "---\nname: test-skill\ndescription: A test skill\ntrigger: /test\nmodel: claude\n---\nDo something with {{args}}.\n").unwrap();

        let loader = SkillLoader::new(dir.path());
        let skills = loader.load_all().unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name(), "test-skill");
        assert_eq!(skills[0].trigger(), "/test");
        assert!(skills[0].template().contains("Do something with"));
    }

    #[test]
    fn ignores_non_md_files() {
        let dir = TempDir::new().unwrap();
        // Write a valid .md skill file
        fs::write(dir.path().join("skill.md"), "---\nname: s\ndescription: d\ntrigger: /s\nmodel: claude\n---\nbody\n").unwrap();
        // Write a non-.md file
        fs::write(dir.path().join("notes.txt"), "ignore me").unwrap();

        let loader = SkillLoader::new(dir.path());
        let skills = loader.load_all().unwrap();

        assert_eq!(skills.len(), 1);
    }
}
