pub const SKILL_DEV_ARCHITECT: &str = include_str!("../skills/dev-architect.md");
pub const SKILL_DEV_TDD: &str = include_str!("../skills/dev-tdd.md");
pub const SKILL_DEV_DEBUG: &str = include_str!("../skills/dev-debug.md");
pub const SKILL_DEV_PR_REVIEW: &str = include_str!("../skills/dev-pr-review.md");
pub const SKILL_CODE_REFACTOR: &str = include_str!("../skills/code-refactor.md");
pub const SKILL_CODE_OPTIMIZE: &str = include_str!("../skills/code-optimize.md");
pub const SKILL_CODE_DOCUMENT: &str = include_str!("../skills/code-document.md");
pub const SKILL_CODE_SECURITY_AUDIT: &str = include_str!("../skills/code-security-audit.md");
pub const SKILL_CODE_TEST_GEN: &str = include_str!("../skills/code-test-gen.md");
pub const SKILL_PROJECT_PLAN: &str = include_str!("../skills/project-plan.md");
pub const SKILL_PROJECT_PRD: &str = include_str!("../skills/project-prd.md");
pub const SKILL_PROJECT_CHANGELOG: &str = include_str!("../skills/project-changelog.md");
pub const SKILL_PROJECT_RELEASE: &str = include_str!("../skills/project-release.md");
pub const SKILL_META_CREATE_AGENT: &str = include_str!("../skills/meta-create-agent.md");
pub const SKILL_META_ANALYZE_CODEBASE: &str = include_str!("../skills/meta-analyze-codebase.md");

pub const ALL_SKILLS: &[(&str, &str)] = &[
    ("dev-architect.md", SKILL_DEV_ARCHITECT),
    ("dev-tdd.md", SKILL_DEV_TDD),
    ("dev-debug.md", SKILL_DEV_DEBUG),
    ("dev-pr-review.md", SKILL_DEV_PR_REVIEW),
    ("code-refactor.md", SKILL_CODE_REFACTOR),
    ("code-optimize.md", SKILL_CODE_OPTIMIZE),
    ("code-document.md", SKILL_CODE_DOCUMENT),
    ("code-security-audit.md", SKILL_CODE_SECURITY_AUDIT),
    ("code-test-gen.md", SKILL_CODE_TEST_GEN),
    ("project-plan.md", SKILL_PROJECT_PLAN),
    ("project-prd.md", SKILL_PROJECT_PRD),
    ("project-changelog.md", SKILL_PROJECT_CHANGELOG),
    ("project-release.md", SKILL_PROJECT_RELEASE),
    ("meta-create-agent.md", SKILL_META_CREATE_AGENT),
    ("meta-analyze-codebase.md", SKILL_META_ANALYZE_CODEBASE),
];
