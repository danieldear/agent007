# Skills Library, Workflow Integration & GitHub Import — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship 15 built-in skills, connect skills to workflows, add GitHub registry browsing and URL import.

**Architecture:** Built-in skills are embedded in the CLI binary via `include_str!` and written during `agent007 init`. The workflow runner resolves `skill:` references by loading the skill template from `.agent007/skills/`. The web dashboard gets a 3-tab Skills page with category grouping, registry browsing, and URL import.

**Tech Stack:** Rust (serde, reqwest), Axum, Vue 3, DaisyUI

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/cli/skills/*.md` (15 new files) | Built-in skill markdown templates |
| `crates/cli/src/built_in_skills.rs` (new) | `include_str!` constants for all 15 skills |
| `crates/cli/src/commands/init.rs` | Write skills during init |
| `crates/skills/src/types.rs` | Add `category` field to `SkillFrontmatter` |
| `crates/workflows/src/types.rs` | Add `skill: Option<String>` to `StepDef`, make `prompt` optional |
| `crates/workflows/src/error.rs` | Add `SkillNotFound` variant |
| `crates/workflows/src/runner.rs` | Resolve skill templates in step execution |
| `crates/web/src/api.rs` | Registry proxy, skill import, single skill get endpoints |
| `crates/web/src/server.rs` | Register new routes |
| `crates/web/Cargo.toml` | Add `reqwest` dependency |
| `crates/web/frontend/src/views/SkillsView.vue` | 3-tab layout with categories, registry, import |
| `crates/web/frontend/src/composables/useApi.js` | Registry and import API methods |

---

### Task 1: Create 15 Built-in Skill Files

**Files:**
- Create: `crates/cli/skills/dev-architect.md`
- Create: `crates/cli/skills/dev-tdd.md`
- Create: `crates/cli/skills/dev-debug.md`
- Create: `crates/cli/skills/dev-pr-review.md`
- Create: `crates/cli/skills/code-refactor.md`
- Create: `crates/cli/skills/code-optimize.md`
- Create: `crates/cli/skills/code-document.md`
- Create: `crates/cli/skills/code-security-audit.md`
- Create: `crates/cli/skills/code-test-gen.md`
- Create: `crates/cli/skills/project-plan.md`
- Create: `crates/cli/skills/project-prd.md`
- Create: `crates/cli/skills/project-changelog.md`
- Create: `crates/cli/skills/project-release.md`
- Create: `crates/cli/skills/meta-create-agent.md`
- Create: `crates/cli/skills/meta-analyze-codebase.md`

- [ ] **Step 1: Create the skills directory and all 15 .md files**

Each file follows this format:

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
- Interfaces and contracts between components
- Data flow and storage design
- Error handling and resilience strategy
- Key trade-offs and alternatives considered
- Deployment and scaling considerations

Requirements:
{{args}}

Previous context (if available):
{{task}}
```

Create all 15 files with appropriate domain-specific prompt templates. Each skill's template should be 10-30 lines of clear, actionable instructions for the agent. Use `{{args}}` for the direct input and `{{task}}` for workflow context.

**Skill templates guidance:**

- `/dev-architect` — system design, components, interfaces, data flow
- `/dev-tdd` — write failing test, implement, refactor cycle
- `/dev-debug` — reproduce, hypothesize, isolate, verify fix
- `/dev-pr-review` — correctness, security, performance, style review
- `/code-refactor` — identify code smells, suggest improvements with before/after
- `/code-optimize` — profiling analysis, bottlenecks, optimization suggestions
- `/code-document` — API docs, architecture docs, inline documentation
- `/code-security-audit` — OWASP check, dependency audit, threat modeling
- `/code-test-gen` — happy path, error cases, boundary conditions, mocking
- `/project-plan` — break into tasks, estimate effort, order by dependencies
- `/project-prd` — goals, user stories, acceptance criteria, constraints
- `/project-changelog` — group by type (feat/fix/docs), link to PRs
- `/project-release` — version strategy, release notes, rollback plan
- `/meta-create-agent` — guided wizard: name, expertise, tools, model, prompt
- `/meta-analyze-codebase` — tech stack, patterns, architecture, entry points

- [ ] **Step 2: Create `crates/cli/src/built_in_skills.rs`**

```rust
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
```

- [ ] **Step 3: Register module in `crates/cli/src/main.rs`**

Add `mod built_in_skills;` to `crates/cli/src/main.rs`.

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p agent007-cli`
Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/skills/ crates/cli/src/built_in_skills.rs crates/cli/src/main.rs
git commit -m "feat(cli): add 15 built-in skill templates embedded in binary"
```

---

### Task 2: Write Built-in Skills During Init

**Files:**
- Modify: `crates/cli/src/commands/init.rs`

- [ ] **Step 1: Add skills seeding section to init**

After section "3. Writing default hooks" and before section "4. Writing built-in workflows", add a new section that writes the built-in skills:

```rust
    // ── 3.5. Seed built-in skills ────────────────────────────────────────────
    section("3.5. Seeding built-in skills");
    let skills_dir = home.join("skills");
    let mut skill_count = 0usize;
    for (filename, content) in crate::built_in_skills::ALL_SKILLS {
        if write_if_missing(&skills_dir.join(filename), content, &format!("skills/{filename}"))? {
            skill_count += 1;
        }
    }
    if skill_count > 0 {
        ok(&format!("{skill_count} built-in skills seeded"));
    }
```

- [ ] **Step 2: Verify init writes skills**

Run: `cargo run -p agent007-cli -- init` in a temp directory, then check `.agent007/skills/` has 15 `.md` files.

- [ ] **Step 3: Verify idempotency**

Run init again — should report "already exists — skipped" for all skills.

- [ ] **Step 4: Commit**

```bash
git add crates/cli/src/commands/init.rs
git commit -m "feat(cli): seed 15 built-in skills during agent007 init"
```

---

### Task 3: Add Category Field to Skill Types

**Files:**
- Modify: `crates/skills/src/types.rs`

- [ ] **Step 1: Add category field**

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    pub trigger: String,
    pub model: String,
    #[serde(default = "default_category")]
    pub category: String,
}

fn default_category() -> String {
    "custom".to_string()
}
```

- [ ] **Step 2: Add accessor to Skill**

```rust
impl Skill {
    // ... existing methods ...
    pub fn category(&self) -> &str { &self.frontmatter.category }
}
```

- [ ] **Step 3: Run skill tests**

Run: `cargo test -p agent007-skills`
Expected: ALL PASS (existing tests don't include `category` in YAML, so the default kicks in).

- [ ] **Step 4: Commit**

```bash
git add crates/skills/src/types.rs
git commit -m "feat(skills): add category field to SkillFrontmatter with 'custom' default"
```

---

### Task 4: Skill-Workflow Integration (Types + Runner)

**Files:**
- Modify: `crates/workflows/src/types.rs`
- Modify: `crates/workflows/src/error.rs`
- Modify: `crates/workflows/src/runner.rs`
- Modify: `crates/workflows/src/dag.rs` (test helper)

- [ ] **Step 1: Add `skill` field and make `prompt` optional in StepDef**

In `crates/workflows/src/types.rs`, change `prompt: String` to `prompt: Option<String>` and add `skill: Option<String>`:

```rust
pub struct StepDef {
    pub id: String,
    pub agent: String,
    pub model: Option<String>,
    pub inputs: Option<Vec<String>>,
    pub depends_on: Option<Vec<String>>,
    pub prompt: Option<String>,           // was: String
    pub skill: Option<String>,            // NEW
    pub output: Option<String>,
    pub requires_approval: Option<bool>,
    #[serde(default, rename = "type")]
    pub r#type: StepType,
    pub evaluate: Option<EvaluateConfig>,
    pub routes: Option<Vec<RouteConfig>>,
}
```

- [ ] **Step 2: Update ALL test StepDef literals**

Every test in `types.rs`, `dag.rs`, and `runner.rs` that constructs a `StepDef` literal needs:
- `prompt: Some("...".to_string())` instead of `prompt: "...".to_string()`
- `skill: None,` added

This includes `make_step` helpers, `simple_def`, `two_step_def`, and all inline `StepDef` constructions.

- [ ] **Step 3: Add SkillNotFound error variant**

In `crates/workflows/src/error.rs`:

```rust
    #[error("skill '{0}' not found in .agent007/skills/")]
    SkillNotFound(String),
```

- [ ] **Step 4: Add skill resolution to runner**

In `crates/workflows/src/runner.rs`, before the Tera rendering, resolve the prompt template:

```rust
let prompt_template = if let Some(skill_trigger) = &step.skill {
    let skills_dir = agent007_core::paths::agent007_home().join("skills");
    let loader = agent007_skills::SkillLoader::new(&skills_dir);
    let skills = loader.load_all().map_err(|e| WorkflowError::StepFailed {
        id: step.id.clone(),
        reason: format!("failed to load skills: {e}"),
    })?;
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

Then use `prompt_template` instead of `step.prompt` in the `render_prompt` call.

Also inject `{{args}}` as an alias for `{{task}}` in `render_prompt`:

```rust
ctx.insert("args", task);  // alias for skill compatibility
```

- [ ] **Step 5: Add workflow YAML test with skill field**

In `types.rs` tests:

```rust
    const SKILL_STEP_YAML: &str = r#"
name = "Skill Test"

[[steps]]
id = "design"
agent = "Architect"
skill = "/dev-architect"
output = "design"
"#;

    #[test]
    fn deserialize_step_with_skill() {
        let def: WorkflowDef = toml::from_str(SKILL_STEP_YAML).unwrap();
        assert_eq!(def.steps[0].skill.as_deref(), Some("/dev-architect"));
        assert!(def.steps[0].prompt.is_none());
    }
```

- [ ] **Step 6: Run all workflow tests**

Run: `cargo test -p agent007-workflows`
Expected: ALL PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/workflows/src/types.rs crates/workflows/src/error.rs crates/workflows/src/runner.rs crates/workflows/src/dag.rs
git commit -m "feat(workflows): skill reference in workflow steps, prompt now optional"
```

---

### Task 5: Web API — Registry Proxy and Import

**Files:**
- Modify: `crates/web/Cargo.toml`
- Modify: `crates/web/src/api.rs`
- Modify: `crates/web/src/server.rs`

- [ ] **Step 1: Add reqwest dependency**

In `crates/web/Cargo.toml`, add:

```toml
reqwest = { version = "0.12", features = ["json"], default-features = false, optional = true }
```

And add a feature:

```toml
[features]
default = ["registry"]
registry = ["reqwest"]
```

This keeps the import/registry feature optional for environments without network.

- [ ] **Step 2: Add skill-registry and import handlers**

In `crates/web/src/api.rs`:

```rust
// ── Skill Registry & Import ──────────────────────────────────────────────────

pub async fn skill_get_handler(
    State(_state): State<AppState>,
    Path(trigger): Path<String>,
) -> impl IntoResponse {
    let skills_dir = agent007_home().join("skills");
    let Ok(entries) = std::fs::read_dir(&skills_dir) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "skills dir not found" }))).into_response();
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(fm) = parse_frontmatter(&content) {
                if fm.get("trigger").and_then(|v| v.as_str()) == Some(&format!("/{trigger}")) {
                    let mut result = fm;
                    if let Some(obj) = result.as_object_mut() {
                        let parts: Vec<&str> = content.splitn(3, "---").collect();
                        if parts.len() >= 3 {
                            obj.insert("template".to_string(), serde_json::Value::String(parts[2].trim().to_string()));
                        }
                    }
                    return Json(result).into_response();
                }
            }
        }
    }

    (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "skill not found" }))).into_response()
}

#[derive(Deserialize)]
pub struct SkillImportRequest {
    pub url: String,
}

pub async fn skill_import_handler(
    State(_state): State<AppState>,
    Json(payload): Json<SkillImportRequest>,
) -> impl IntoResponse {
    let url = normalize_github_url(&payload.url);

    #[cfg(feature = "registry")]
    {
        let client = reqwest::Client::new();
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
        };

        if !resp.status().is_success() {
            return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": format!("HTTP {}", resp.status()) }))).into_response();
        }

        let content = match resp.text().await {
            Ok(t) => t,
            Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
        };

        if content.len() > 100_000 {
            return (StatusCode::PAYLOAD_TOO_LARGE, Json(serde_json::json!({ "error": "skill file exceeds 100KB limit" }))).into_response();
        }

        // Validate frontmatter
        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid skill: missing frontmatter" }))).into_response();
        }

        #[derive(serde::Deserialize)]
        struct MinFm { trigger: String }
        let fm: MinFm = match serde_yaml::from_str(parts[1]) {
            Ok(f) => f,
            Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": format!("invalid frontmatter: {e}") }))).into_response(),
        };

        let filename = fm.trigger.trim_start_matches('/')
            .chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect::<String>();
        let skills_dir = agent007_home().join("skills");
        let _ = std::fs::create_dir_all(&skills_dir);
        let path = skills_dir.join(format!("{filename}.md"));

        match std::fs::write(&path, &content) {
            Ok(()) => Json(serde_json::json!({ "ok": true, "trigger": fm.trigger, "path": path.display().to_string() })).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
        }
    }

    #[cfg(not(feature = "registry"))]
    {
        (StatusCode::NOT_IMPLEMENTED, Json(serde_json::json!({ "error": "registry feature not enabled" }))).into_response()
    }
}

pub async fn skill_registry_handler() -> impl IntoResponse {
    #[cfg(feature = "registry")]
    {
        let registry_url = "https://raw.githubusercontent.com/agent007-community/skills/main/registry.json";
        let client = reqwest::Client::new();
        match client.get(registry_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(val) => Json(val).into_response(),
                    Err(e) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
                }
            }
            Ok(resp) => {
                // Registry doesn't exist yet — return empty array
                Json(serde_json::json!([])).into_response()
            }
            Err(_) => {
                // Offline — return empty array
                Json(serde_json::json!([])).into_response()
            }
        }
    }

    #[cfg(not(feature = "registry"))]
    {
        Json(serde_json::json!([])).into_response()
    }
}

fn normalize_github_url(url: &str) -> String {
    let url = url.trim();
    if url.contains("github.com") && url.contains("/blob/") {
        url.replace("github.com", "raw.githubusercontent.com").replace("/blob/", "/")
    } else {
        url.to_string()
    }
}
```

- [ ] **Step 3: Register routes in server.rs**

Add to `into_router()`:

```rust
            .route("/api/skills/{trigger}", get(api::skill_get_handler))
            .route("/api/skills/import", post(api::skill_import_handler))
            .route("/api/skill-registry", get(api::skill_registry_handler))
```

- [ ] **Step 4: Update skills_handler to include category**

Update the `parse_frontmatter` helper or the `skills_handler` to also return the `category` field.

- [ ] **Step 5: Run web tests**

Run: `cargo test -p agent007-web`
Expected: ALL PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/web/Cargo.toml crates/web/src/api.rs crates/web/src/server.rs
git commit -m "feat(web): add skill registry, import, and single-skill get API endpoints"
```

---

### Task 6: Update useApi.js

**Files:**
- Modify: `crates/web/frontend/src/composables/useApi.js`

- [ ] **Step 1: Add registry and import methods**

Add to the `api` object after `saveSkill`:

```javascript
    getSkill: (trigger) => fetchJson(`/api/skills/${encodeURIComponent(trigger)}`),
    importSkill: (url) => fetchJson('/api/skills/import', { method: 'POST', body: JSON.stringify({ url }) }),
    getRegistry: () => fetchJson('/api/skill-registry'),
```

- [ ] **Step 2: Commit**

```bash
git add crates/web/frontend/src/composables/useApi.js
git commit -m "feat(web): add registry, import, and skill get methods to useApi"
```

---

### Task 7: Redesign SkillsView.vue

**Files:**
- Modify: `crates/web/frontend/src/views/SkillsView.vue`

- [ ] **Step 1: Replace SkillsView.vue with 3-tab layout**

The new view has:

**Tab 1: Installed** — Skills grouped by category (`dev`, `code`, `project`, `meta`, `custom`). Each category is a collapsible section. Each card shows name, trigger badge, description. Cards have Edit/Delete actions.

**Tab 2: Browse** — Fetches from `/api/skill-registry`. Category filter pills, search input, install button per card. Shows "Installed" badge if trigger already exists locally.

**Tab 3: Import** — URL text input, "Import" button, preview panel. On submit, calls `POST /api/skills/import`, refreshes installed list.

Remove the 4 hardcoded quick templates — they're superseded by built-in skills.

Keep the "+ New Skill" button for manual creation.

The grouping logic:
```javascript
const grouped = computed(() => {
  const groups = {}
  for (const s of skills.value) {
    const cat = s.category || (s.trigger?.split('-')[0]?.replace('/', '') || 'custom')
    if (!groups[cat]) groups[cat] = []
    groups[cat].push(s)
  }
  return groups
})
```

- [ ] **Step 2: Commit**

```bash
git add crates/web/frontend/src/views/SkillsView.vue
git commit -m "feat(web): redesign Skills page with category tabs, registry browser, and URL import"
```

---

### Task 8: Build Frontend and Verify Full Stack

**Files:**
- Working in: `crates/web/frontend/`

- [ ] **Step 1: Build frontend**

```bash
cd crates/web/frontend && npm run build
```

Expected: Build succeeds.

- [ ] **Step 2: Run cargo check**

```bash
cargo check --workspace
```

Expected: No errors.

- [ ] **Step 3: Run all tests**

```bash
cargo test -p agent007-workflows -p agent007-web -p agent007-skills
```

Expected: ALL PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/web/static/dist/ crates/web/frontend/
git commit -m "build: rebuild frontend with skills library, registry, and import UI"
```

---

## Self-Review Checklist

- [x] **Spec coverage**: All spec sections mapped to tasks:
  - Built-in skills → Task 1 + Task 2
  - Category field → Task 3
  - Skill-workflow integration → Task 4
  - Registry + import API → Task 5
  - Frontend API methods → Task 6
  - Dashboard redesign → Task 7
  - Build verification → Task 8
- [x] **Placeholder scan**: No TBD/TODO found
- [x] **Type consistency**: `SkillFrontmatter.category`, `StepDef.skill`, `SkillNotFound` used consistently
- [x] **Backward compatibility**: `prompt` becomes `Option<String>` — all test literals need updating. `category` defaults to "custom" for existing skills without it.
