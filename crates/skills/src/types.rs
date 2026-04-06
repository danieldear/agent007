use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    pub trigger: String,
    pub model: String,
    #[serde(default = "default_category")]
    pub category: String,
    /// Semantic version of the skill. Increment to signal template or behavior changes.
    #[serde(default = "default_version")]
    pub version: String,
    /// Optional tags for grouping / filtering skills (e.g. ["security", "review"]).
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_category() -> String {
    "custom".to_string()
}

fn default_version() -> String {
    "1.0.0".to_string()
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub frontmatter: SkillFrontmatter,
    pub template: String,
}

impl Skill {
    pub fn name(&self) -> &str { &self.frontmatter.name }
    pub fn trigger(&self) -> &str { &self.frontmatter.trigger }
    pub fn model(&self) -> &str { &self.frontmatter.model }
    pub fn template(&self) -> &str { &self.template }
    pub fn category(&self) -> &str { &self.frontmatter.category }
    pub fn version(&self) -> &str { &self.frontmatter.version }
    pub fn tags(&self) -> &[String] { &self.frontmatter.tags }
}
