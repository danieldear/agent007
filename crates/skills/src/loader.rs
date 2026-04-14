use crate::error::SkillError;
use crate::types::{Skill, SkillFrontmatter};
use std::path::{Path, PathBuf};

pub struct SkillLoader {
    skills_dir: PathBuf,
}

impl SkillLoader {
    pub fn new(skills_dir: impl Into<PathBuf>) -> Self {
        Self {
            skills_dir: skills_dir.into(),
        }
    }

    pub fn load_all(&self) -> Result<Vec<Skill>, SkillError> {
        let mut skills = Vec::new();

        let entries = std::fs::read_dir(&self.skills_dir).map_err(|e| SkillError::Io {
            path: self.skills_dir.clone(),
            source: e,
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| SkillError::Io {
                path: self.skills_dir.clone(),
                source: e,
            })?;
            if let Some(skill) = self.load_entry(&entry.path())? {
                skills.push(skill);
            }
        }

        Ok(skills)
    }

    fn load_entry(&self, entry_path: &Path) -> Result<Option<Skill>, SkillError> {
        if entry_path.is_file() {
            if entry_path.extension().and_then(|e| e.to_str()) != Some("md") {
                return Ok(None);
            }
            return self
                .load_skill_manifest(entry_path, entry_path, &self.skills_dir)
                .map(Some);
        }

        if entry_path.is_dir() {
            let manifest_path = entry_path.join("SKILL.md");
            if !manifest_path.is_file() {
                return Ok(None);
            }
            return self
                .load_skill_manifest(&manifest_path, entry_path, entry_path)
                .map(Some);
        }

        Ok(None)
    }

    fn load_skill_manifest(
        &self,
        manifest_path: &Path,
        entry_path: &Path,
        skill_dir: &Path,
    ) -> Result<Skill, SkillError> {
        let content = std::fs::read_to_string(manifest_path).map_err(|e| SkillError::Io {
            path: manifest_path.to_path_buf(),
            source: e,
        })?;

        // Split on "---" to get frontmatter
        // skill files start with "---\n" so splitn(3, "---") yields ["", " fm\n", " body\n"]
        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            return Err(SkillError::MissingFrontmatter {
                path: manifest_path.to_path_buf(),
            });
        }

        let frontmatter: SkillFrontmatter =
            serde_yaml::from_str(parts[1]).map_err(|e| SkillError::FrontmatterParse {
                path: manifest_path.to_path_buf(),
                source: e,
            })?;

        let template = parts[2].trim().to_string();
        Ok(Skill {
            frontmatter,
            template,
            manifest_path: manifest_path.to_path_buf(),
            entry_path: entry_path.to_path_buf(),
            skill_dir: skill_dir.to_path_buf(),
        })
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
        assert_eq!(skills[0].manifest_path(), skill_path.as_path());
        assert_eq!(skills[0].entry_path(), skill_path.as_path());
        assert_eq!(skills[0].skill_dir(), dir.path());
        assert!(!skills[0].is_package());
    }

    #[test]
    fn ignores_non_md_files() {
        let dir = TempDir::new().unwrap();
        // Write a valid .md skill file
        fs::write(
            dir.path().join("skill.md"),
            "---\nname: s\ndescription: d\ntrigger: /s\nmodel: claude\n---\nbody\n",
        )
        .unwrap();
        // Write a non-.md file
        fs::write(dir.path().join("notes.txt"), "ignore me").unwrap();

        let loader = SkillLoader::new(dir.path());
        let skills = loader.load_all().unwrap();

        assert_eq!(skills.len(), 1);
    }

    #[test]
    fn loads_skill_directory_package() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = dir.path().join("review-skill");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("SKILL.md"),
            "---\nname: review\ndescription: packaged skill\ntrigger: /review\nmodel: claude\n---\nUse {{skill_dir}}.\n",
        )
        .unwrap();

        let loader = SkillLoader::new(dir.path());
        let skills = loader.load_all().unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].trigger(), "/review");
        assert_eq!(
            skills[0].manifest_path(),
            pkg_dir.join("SKILL.md").as_path()
        );
        assert_eq!(skills[0].entry_path(), pkg_dir.as_path());
        assert_eq!(skills[0].skill_dir(), pkg_dir.as_path());
        assert!(skills[0].is_package());
    }

    #[test]
    fn ignores_directory_without_skill_manifest() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("empty-package")).unwrap();
        fs::write(
            dir.path().join("flat.md"),
            "---\nname: flat\ndescription: d\ntrigger: /flat\nmodel: claude\n---\nbody\n",
        )
        .unwrap();

        let loader = SkillLoader::new(dir.path());
        let skills = loader.load_all().unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].trigger(), "/flat");
    }
}
