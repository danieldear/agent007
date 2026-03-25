use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    pub trigger: String,
    pub model: String,
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
}
