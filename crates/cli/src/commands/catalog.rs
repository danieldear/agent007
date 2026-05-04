use super::run::{agent007_global_home, agent007_project_home};
use crate::config::Config;
use anyhow::Result;
use clap::Subcommand;
use clap::ValueEnum;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Subcommand, Debug)]
pub enum CatalogAction {
    /// Audit skills, workflows, and personas for collisions and quality-contract gaps
    Audit {
        /// Scope to audit: project, global, or both
        #[arg(long, value_enum, default_value_t = CatalogScope::Project)]
        scope: CatalogScope,
        /// Return machine-readable JSON
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Exit non-zero if any warnings are found (errors already fail by default)
        #[arg(long, default_value_t = false)]
        fail_on_warn: bool,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum CatalogScope {
    Project,
    Global,
    Both,
}

impl CatalogScope {
    fn as_str(self) -> &'static str {
        match self {
            CatalogScope::Project => "project",
            CatalogScope::Global => "global",
            CatalogScope::Both => "both",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Severity {
    Error,
    Warn,
}

#[derive(Debug, Clone, Serialize)]
struct Finding {
    severity: Severity,
    kind: String,
    id: String,
    source: String,
    message: String,
}

#[derive(Debug, Default, Serialize)]
struct AuditReport {
    skills_loaded: usize,
    workflows_loaded: usize,
    personas_loaded: usize,
    findings: Vec<Finding>,
}

impl AuditReport {
    fn push(
        &mut self,
        severity: Severity,
        kind: impl Into<String>,
        id: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.findings.push(Finding {
            severity,
            kind: kind.into(),
            id: id.into(),
            source: source.into(),
            message: message.into(),
        });
    }

    fn errors(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| matches!(f.severity, Severity::Error))
            .count()
    }

    fn warnings(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| matches!(f.severity, Severity::Warn))
            .count()
    }
}

pub async fn execute(_config: Arc<Config>, action: CatalogAction) -> Result<()> {
    match action {
        CatalogAction::Audit {
            scope,
            json,
            fail_on_warn,
        } => {
            let report = build_report(scope);
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_human_report(scope, &report);
            }

            let has_errors = report.errors() > 0;
            let has_warnings = report.warnings() > 0;
            if has_errors || (fail_on_warn && has_warnings) {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn configured_dirs_for_scope(kind: &str, scope: CatalogScope) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match scope {
        CatalogScope::Project => {
            if let Some(project_home) = agent007_project_home() {
                dirs.push(project_home.join(kind));
            }
        }
        CatalogScope::Global => {
            dirs.push(agent007_global_home().join(kind));
        }
        CatalogScope::Both => {
            if let Some(project_home) = agent007_project_home() {
                dirs.push(project_home.join(kind));
            }
            let global = agent007_global_home().join(kind);
            if !dirs.iter().any(|d| d == &global) {
                dirs.push(global);
            }
        }
    }
    dirs
}

fn is_probably_semver(raw: &str) -> bool {
    let raw = raw.trim();
    if raw.is_empty() {
        return false;
    }
    let without_build = raw.split_once('+').map(|(left, _)| left).unwrap_or(raw);
    let core = without_build
        .split_once('-')
        .map(|(left, _)| left)
        .unwrap_or(without_build);
    let mut parts = core.split('.');
    let major = parts.next();
    let minor = parts.next();
    let patch = parts.next();
    if parts.next().is_some() {
        return false;
    }
    [major, minor, patch]
        .into_iter()
        .all(|p| p.is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())))
}

fn build_report(scope: CatalogScope) -> AuditReport {
    let mut report = AuditReport::default();
    audit_skills(scope, &mut report);
    audit_workflows(scope, &mut report);
    audit_personas(scope, &mut report);
    report
}

fn audit_skills(scope: CatalogScope, report: &mut AuditReport) {
    let dirs = configured_dirs_for_scope("skills", scope);
    let mut trigger_sources: HashMap<String, Vec<String>> = HashMap::new();

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }
        let loader = agent007_skills::SkillLoader::new(dir);
        match loader.load_all() {
            Ok(skills) => {
                report.skills_loaded += skills.len();
                for skill in skills {
                    let trigger = skill.trigger().to_string();
                    trigger_sources
                        .entry(trigger.clone())
                        .or_default()
                        .push(skill.manifest_path().display().to_string());

                    if !trigger.starts_with('/') {
                        report.push(
                            Severity::Warn,
                            "skill",
                            trigger.clone(),
                            skill.manifest_path().display().to_string(),
                            "trigger should start with '/' for slash-command consistency",
                        );
                    }

                    if skill.frontmatter.description.trim().len() < 24 {
                        report.push(
                            Severity::Warn,
                            "skill",
                            trigger.clone(),
                            skill.manifest_path().display().to_string(),
                            "description is short; add a clear purpose, scope, and expected output",
                        );
                    }

                    if skill.frontmatter.tags.is_empty() {
                        report.push(
                            Severity::Warn,
                            "skill",
                            trigger.clone(),
                            skill.manifest_path().display().to_string(),
                            "missing tags; add tags for routing and discovery",
                        );
                    }

                    if !is_probably_semver(skill.version()) {
                        report.push(
                            Severity::Warn,
                            "skill",
                            trigger.clone(),
                            skill.manifest_path().display().to_string(),
                            "version is not semver-like (expected e.g. 1.2.0)",
                        );
                    }

                    let template = skill.template().to_ascii_lowercase();
                    if !template.contains("return exactly") && !template.contains("output format") {
                        report.push(
                            Severity::Warn,
                            "skill",
                            trigger,
                            skill.manifest_path().display().to_string(),
                            "template has no explicit output contract ('Return exactly...' or equivalent)",
                        );
                    }
                }
            }
            Err(err) => report.push(
                Severity::Error,
                "skill",
                dir.display().to_string(),
                dir.display().to_string(),
                format!("failed to load skills: {err}"),
            ),
        }
    }

    for (trigger, sources) in trigger_sources {
        if sources.len() > 1 {
            report.push(
                Severity::Warn,
                "skill-collision",
                trigger,
                sources.join(" | "),
                "trigger collides across homes; project/global override may hide another skill",
            );
        }
    }
}

fn audit_workflows(scope: CatalogScope, report: &mut AuditReport) {
    let dirs = configured_dirs_for_scope("workflows", scope);
    let mut name_sources: HashMap<String, Vec<String>> = HashMap::new();

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }

        let loader = agent007_workflows::WorkflowLoader::new(dir.clone());
        let names = match loader.list_names() {
            Ok(n) => n,
            Err(err) => {
                report.push(
                    Severity::Error,
                    "workflow",
                    dir.display().to_string(),
                    dir.display().to_string(),
                    format!("failed to list workflows: {err}"),
                );
                continue;
            }
        };

        for name in names {
            let path = resolve_workflow_path(dir, &name);
            name_sources
                .entry(name.clone())
                .or_default()
                .push(path.display().to_string());

            let def = match loader.load_named(&name) {
                Ok(d) => d,
                Err(err) => {
                    report.push(
                        Severity::Error,
                        "workflow",
                        name.clone(),
                        path.display().to_string(),
                        format!("failed to load workflow: {err}"),
                    );
                    continue;
                }
            };

            report.workflows_loaded += 1;

            if def.description.as_deref().unwrap_or(" ").trim().is_empty() {
                report.push(
                    Severity::Warn,
                    "workflow",
                    name.clone(),
                    path.display().to_string(),
                    "missing description; add objective, scope, and exit criteria",
                );
            }

            if def.reliability.is_none() {
                report.push(
                    Severity::Warn,
                    "workflow",
                    name.clone(),
                    path.display().to_string(),
                    "missing reliability policy block",
                );
            }

            if def.eval_gate.is_none() {
                report.push(
                    Severity::Warn,
                    "workflow",
                    name.clone(),
                    path.display().to_string(),
                    "missing eval_gate policy block",
                );
            }

            let mut has_approval_step = false;
            for step in &def.steps {
                if step.requires_approval == Some(true) {
                    has_approval_step = true;
                }
                match step.r#type {
                    agent007_workflows::types::StepType::Router => {}
                    _ => {
                        if step.output.as_deref().unwrap_or(" ").trim().is_empty() {
                            report.push(
                                Severity::Warn,
                                "workflow-step",
                                format!("{}:{}", name, step.id),
                                path.display().to_string(),
                                "step is missing output variable; downstream composition becomes brittle",
                            );
                        }
                    }
                }
            }

            if !has_approval_step {
                report.push(
                    Severity::Warn,
                    "workflow",
                    name,
                    path.display().to_string(),
                    "no approval step found; add at least one human gate for high-risk flows",
                );
            }
        }
    }

    for (name, sources) in name_sources {
        if sources.len() > 1 {
            report.push(
                Severity::Warn,
                "workflow-collision",
                name,
                sources.join(" | "),
                "workflow name collides across homes; project/global override may hide another workflow",
            );
        }
    }
}

fn resolve_workflow_path(dir: &Path, name: &str) -> PathBuf {
    for ext in ["yaml", "yml", "toml"] {
        let candidate = dir.join(format!("{name}.{ext}"));
        if candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{name}.yaml"))
}

fn audit_personas(scope: CatalogScope, report: &mut AuditReport) {
    let dirs = configured_dirs_for_scope("personas", scope);
    let registry =
        agent007_personas::PersonaRegistry::load_from_dirs(dirs.iter().map(|d| d.as_path()));
    let registry = match registry {
        Ok(r) => r,
        Err(err) => {
            report.push(
                Severity::Error,
                "persona",
                "registry".to_string(),
                "configured dirs".to_string(),
                format!("failed to load persona registry: {err}"),
            );
            return;
        }
    };

    use agent007_core::PersonaProvider;
    let personas = registry.list();
    report.personas_loaded = personas.len();

    for persona in personas {
        if persona.description.trim().len() < 20 {
            report.push(
                Severity::Warn,
                "persona",
                persona.name.clone(),
                "registry".to_string(),
                "description is too short; clarify role boundary and ownership",
            );
        }

        if persona.allowed_tools.is_empty() {
            report.push(
                Severity::Warn,
                "persona",
                persona.name.clone(),
                "registry".to_string(),
                "allowed_tools is empty; execution constraints are unclear",
            );
        }

        if persona.preferred_model.trim().is_empty() {
            report.push(
                Severity::Error,
                "persona",
                persona.name.clone(),
                "registry".to_string(),
                "preferred_model is empty",
            );
        }

        if persona.system_prompt.trim().len() < 180 {
            report.push(
                Severity::Warn,
                "persona",
                persona.name,
                "registry".to_string(),
                "system prompt is short; add stronger decision rules and output expectations",
            );
        }
    }
}

fn print_human_report(scope: CatalogScope, report: &AuditReport) {
    println!("agent007 catalog audit");
    println!("────────────────────");
    println!("scope:            {}", scope.as_str());
    println!("skills loaded:    {}", report.skills_loaded);
    println!("workflows loaded: {}", report.workflows_loaded);
    println!("personas loaded:  {}", report.personas_loaded);
    println!(
        "findings:         {} errors, {} warnings",
        report.errors(),
        report.warnings()
    );

    if report.findings.is_empty() {
        println!("\nNo findings.");
        return;
    }

    println!("\nFindings:");
    for finding in &report.findings {
        let sev = match finding.severity {
            Severity::Error => "ERROR",
            Severity::Warn => "WARN",
        };
        println!(
            "- [{sev}] {}:{}\n  source: {}\n  {}",
            finding.kind, finding.id, finding.source, finding.message
        );
    }
}

#[cfg(test)]
mod tests {
    use super::is_probably_semver;

    #[test]
    fn semver_like_values_are_accepted() {
        assert!(is_probably_semver("1.0.0"));
        assert!(is_probably_semver("2.11.3-beta.1"));
        assert!(is_probably_semver("0.1.0+build.12"));
    }

    #[test]
    fn invalid_semver_like_values_are_rejected() {
        assert!(!is_probably_semver("1.0"));
        assert!(!is_probably_semver("v1.0.0"));
        assert!(!is_probably_semver("1.0.x"));
    }
}
