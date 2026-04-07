# Skills Library, Workflow Integration & GitHub Import

**Date**: 2026-03-26
**Status**: Draft → Awaiting Review

## Problem

agent007 ships with zero pre-installed skills. The Skills dashboard shows "No skills installed yet" with only 4 hardcoded client-side templates that don't persist. Users have no way to discover or import community skills. Workflow steps use inline prompts instead of reusable skill templates, leading to duplication and poor maintainability.

## Goals

1. Ship 15 built-in skills across 4 categories so agent007 works out of the box
2. Connect skills to workflows — steps reference skills instead of inline prompts
3. Add an official skill registry (GitHub-hosted) for browsable, one-click install
4. Support importing skills from any GitHub URL
5. Restructure the Skills dashboard with categorized views and import UI

## Non-Goals

- Skill versioning or update tracking
- Publishing/sharing skills from the dashboard
- Marketplace with ratings or reviews
- Skill execution from the dashboard (already exists)

---

## Design

### 1. Built-in Skills Library

15 skills across 4 namespaced categories, pre-installed during `agent007 init`. Each skill is a `.md` file with YAML frontmatter, written to `.agent007/skills/`.

#### Category: dev (Development Workflows)

| Trigger | Name | Purpose |
|---|---|---|
| `/dev-architect` | Architect | System design from requirements — components, interfaces, data flow, trade-offs |
| `/dev-tdd` | TDD | Test-driven development — write failing test, implement minimal code, refactor |
| `/dev-debug` | Debug | Systematic debugging — reproduce, hypothesize, isolate, verify fix |
| `/dev-pr-review` | PR Review | Pull request review — correctness, security, performance, style |

#### Category: code (Code Operations)

| Trigger | Name | Purpose |
|---|---|---|
| `/code-refactor` | Refactor | Analyze code and suggest refactoring with before/after examples |
| `/code-optimize` | Optimize | Performance analysis — profiling, bottleneck identification, optimization |
| `/code-document` | Document | Generate documentation — API docs, architecture docs, inline comments |
| `/code-security-audit` | Security Audit | Security vulnerability analysis — OWASP, dependency audit, remediation |
| `/code-test-gen` | Test Generator | Generate comprehensive test suites — happy path, error cases, boundaries |

#### Category: project (Project Management)

| Trigger | Name | Purpose |
|---|---|---|
| `/project-plan` | Plan | Break features into tasks with effort estimates and dependency ordering |
| `/project-prd` | PRD | Write a product requirements document from ideas and constraints |
| `/project-changelog` | Changelog | Generate changelog from git history — grouped by type, linked to PRs |
| `/project-release` | Release | Release planning — version strategy, release notes, rollback plan |

#### Category: meta (Agent Meta-Skills)

| Trigger | Name | Purpose |
|---|---|---|
| `/meta-create-agent` | Create Agent | Guided wizard to define a new persona — role, expertise, tools, model |
| `/meta-analyze-codebase` | Analyze Codebase | Project structure analysis — tech stack, patterns, architecture, entry points |

#### Skill file format

Each built-in skill follows the existing `.md` format:

```markdown
---
name: Architect
trigger: /dev-architect
description: System design from requirements
model: claude-sonnet-4-6
category: dev
---
You are a software architect. Given the following requirements, design a system architecture.

Cover:
- Component breakdown with clear responsibilities
- Interfaces between components
- Data flow and storage
- Error handling strategy
- Key trade-offs and alternatives considered

Requirements:
{{args}}

Previous context (if available):
{{task}}
```

The `category` field in frontmatter is new — it enables dashboard grouping. Existing skills without a category default to "custom".

#### Init behavior

`agent007 init` writes all 15 `.md` files to `.agent007/skills/` unless a file with the same trigger already exists (no overwrite). This is idempotent — running init again won't clobber user edits.

The skill file contents are embedded in the `agent007` binary (compiled into the `cli` crate as string constants or via `include_str!`), not fetched from the network.

### 2. Skill-Workflow Integration

#### StepDef changes

Add `skill: Option<String>` to `StepDef` in `crates/workflows/src/types.rs`:

```rust
pub struct StepDef {
    pub id: String,
    pub agent: String,
    pub model: Option<String>,
    pub inputs: Option<Vec<String>>,
    pub depends_on: Option<Vec<String>>,
    pub prompt: Option<String>,           // now Optional
    pub skill: Option<String>,            // NEW: skill trigger
    pub output: Option<String>,
    pub requires_approval: Option<bool>,
    pub r#type: StepType,
    pub evaluate: Option<EvaluateConfig>,
    pub routes: Option<Vec<RouteConfig>>,
}
```

Validation: a step must have either `prompt` or `skill` set. If both are set, `skill` takes precedence. If neither is set, validation fails.

Note: Making `prompt` optional is a breaking change for struct literals in tests. All test `StepDef` literals need updating to `prompt: Some("...")` or `prompt: None`. This is the same pattern used when adding `StepType` in the orchestration patterns work.

#### Runner changes

In `WorkflowRunner::run()`, before rendering the prompt:

1. If `step.skill` is set, load the skill from `.agent007/skills/` using `SkillLoader`
2. Use the skill's template as the prompt
3. Auto-inject all available context:
   - `{{task}}` = original task input
   - `{{args}}` = original task input (alias for skill compatibility)
   - All outputs from prior steps (e.g., `{{design}}`, `{{code}}`)
4. If the step also has explicit `inputs`, those values override the auto-injected ones
5. Render the final prompt through Tera and send to the model

```rust
let prompt_template = if let Some(skill_trigger) = &step.skill {
    let skills_dir = agent007_core::paths::agent007_home().join("skills");
    let loader = agent007_skills::SkillLoader::new(&skills_dir);
    let skills = loader.load_all()?;
    let skill = skills.into_iter()
        .find(|s| s.trigger() == skill_trigger)
        .ok_or_else(|| WorkflowError::SkillNotFound(skill_trigger.clone()))?;
    skill.template().to_string()
} else if let Some(prompt) = &step.prompt {
    prompt.clone()
} else {
    return Err(WorkflowError::StepFailed {
        id: step.id.clone(),
        reason: "step must have either 'prompt' or 'skill'".to_string(),
    });
};
```

Add a new error variant:
```rust
#[error("skill '{0}' not found in .agent007/skills/")]
SkillNotFound(String),
```

#### Workflow YAML example

```yaml
name: full-feature-pipeline
description: End-to-end feature development

steps:
  - id: brainstorm
    agent: Architect
    skill: /dev-architect
    output: architecture

  - id: plan
    agent: Architect
    skill: /project-plan
    output: plan
    depends_on: [brainstorm]

  - id: implement
    agent: Coder
    skill: /dev-tdd
    output: code
    depends_on: [plan]

  - id: review
    type: evaluator
    agent: CodeReviewer
    skill: /dev-pr-review
    output: review_result
    evaluate:
      decision_field: verdict
      on_pass: security
      on_fail: implement
      max_retries: 3
    depends_on: [implement]

  - id: security
    agent: SecurityReviewer
    skill: /code-security-audit
    output: security_report
    depends_on: [review]

  - id: release
    agent: DevOpsEngineer
    skill: /project-release
    output: release_plan
    depends_on: [security]
```

### 3. GitHub Import: Official Registry

#### Registry structure

An official GitHub repo (`agent007-community/skills`) hosts:

```
agent007-community/skills/
├── registry.json          # Index of all available skills
├── dev/
│   ├── frontend-design.md
│   ├── backend-design.md
│   └── api-design.md
├── code/
│   ├── rust-patterns.md
│   └── react-patterns.md
├── project/
│   └── sprint-planning.md
└── community/
    └── ...
```

`registry.json` schema:

```json
[
  {
    "name": "Frontend Designer",
    "trigger": "/dev-frontend-design",
    "category": "dev",
    "description": "Design UI components with accessibility and responsiveness",
    "file": "dev/frontend-design.md",
    "author": "agent007",
    "tags": ["ui", "design", "accessibility"]
  }
]
```

#### API endpoint

`GET /api/skill-registry` — fetches the registry index from GitHub (with in-memory cache, TTL 5 minutes):

```
GET https://raw.githubusercontent.com/agent007-community/skills/main/registry.json
```

Returns the registry JSON to the frontend. The frontend displays it in the Browse tab.

Install action: `POST /api/skills/import` with `{ "url": "<raw file URL>" }` — downloads the `.md` file and saves to `.agent007/skills/`.

### 4. GitHub Import: Custom URL

#### API endpoint

`POST /api/skills/import` handles:

1. **Single file URL** (e.g., `https://github.com/user/repo/blob/main/skills/my-skill.md`):
   - Converts to raw URL
   - Downloads the `.md` file
   - Validates frontmatter (must have `name`, `trigger`, `description`, `model`)
   - Saves to `.agent007/skills/<trigger-sanitized>.md`

2. **Repo URL** (e.g., `https://github.com/user/repo`):
   - Fetches the repo's `/skills` directory listing via GitHub API
   - Returns the list of `.md` files found
   - User selects which to install

3. **Shorthand** (e.g., `user/repo`):
   - Resolves to `https://github.com/user/repo`

#### Security

- Skills are text-only (markdown + YAML frontmatter). No executable code.
- File size cap: 100KB per skill file.
- Frontmatter validation: reject files without required fields.
- Preview before save: the import UI shows the skill's content before committing.
- No automatic execution on import.

### 5. Updated Skills Dashboard UI

#### Tab 1: Installed (default)

Skills grouped by category prefix. Each category is a collapsible section:

```
▼ dev (4 skills)
  [/dev-architect] Architect — System design from requirements
  [/dev-tdd] TDD — Test-driven development
  [/dev-debug] Debug — Systematic debugging
  [/dev-pr-review] PR Review — Pull request review

▼ code (5 skills)
  ...

▼ custom (user-created)
  ...
```

Each skill card: name, trigger badge, description, Edit/Delete actions.

The 4 hardcoded quick templates are removed — replaced by the built-in skills.

#### Tab 2: Browse Registry

- Category filter sidebar (pills/tags)
- Search input
- Grid of skill cards from the registry
- Each card: name, description, author, tags, "Install" button
- Install state: shows "Installed" badge if trigger already exists locally

#### Tab 3: Import

- URL input field with "Import" button
- Detects URL type (single file vs repo) and shows appropriate UI
- Preview panel showing the `.md` content before saving
- Recent imports list (last 5, stored in memory only)

"+ New Skill" button remains for manual creation.

---

## Files Changed

| Area | File | Change |
|---|---|---|
| Types | `crates/workflows/src/types.rs` | Add `skill: Option<String>`, make `prompt` optional |
| Runner | `crates/workflows/src/runner.rs` | Skill template resolution, auto-inject context |
| Error | `crates/workflows/src/error.rs` | `SkillNotFound` variant |
| Skills types | `crates/skills/src/types.rs` | Add `category` field to `SkillFrontmatter` |
| Init | `crates/cli/src/commands/init.rs` | Write 15 built-in skill files during init |
| Built-in skills | `crates/cli/src/built_in_skills.rs` (new) | 15 skill templates as `include_str!` or constants |
| Skill files | `crates/cli/skills/` (new, 15 `.md` files) | Actual skill markdown files embedded in binary |
| Web API | `crates/web/src/api.rs` | `GET /api/skill-registry`, `POST /api/skills/import`, `GET /api/skills/{trigger}` |
| Web Server | `crates/web/src/server.rs` | Register new routes |
| Web Cargo | `crates/web/Cargo.toml` | Add `reqwest` for HTTP fetching (registry + import) |
| Vue: Skills | `crates/web/frontend/src/views/SkillsView.vue` | 3-tab layout, category groups, registry browser, import UI |
| Vue: API | `crates/web/frontend/src/composables/useApi.js` | `getRegistry()`, `importSkill(url)`, `getSkill(trigger)` |

## Testing

- **Types**: Deserialize YAML with `skill` field, without `prompt` field
- **Types**: Deserialize YAML with both `skill` and `prompt` (skill takes precedence)
- **Types**: Backward compat — existing YAML with only `prompt` still works
- **Runner**: Step with `skill` loads correct template and renders prompt
- **Runner**: Step with missing skill returns `SkillNotFound` error
- **Runner**: Skill template receives all prior step outputs as context
- **Init**: Running init creates 15 `.md` files in `.agent007/skills/`
- **Init**: Running init again doesn't overwrite existing skills
- **API**: `GET /api/skill-registry` returns array (mock in tests)
- **API**: `POST /api/skills/import` with valid URL saves file
- **API**: `POST /api/skills/import` with oversized file returns error
- **Skills**: Category field parsed from frontmatter, defaults to "custom"
- **Vue**: Installed tab groups skills by category
- **Vue**: Browse tab displays registry with install buttons
- **Vue**: Import tab downloads and previews before saving
