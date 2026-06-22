use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use super::run::agent007_global_home;

#[derive(Debug, Clone)]
pub struct SlashCommandSyncSummary {
    pub commands_dir: PathBuf,
    pub written: usize,
    pub removed: usize,
    pub skill_commands: usize,
    pub workflow_commands: usize,
}

pub fn sync_claude_slash_commands_for_home(write_home: &Path) -> Result<SlashCommandSyncSummary> {
    let commands_dir = claude_commands_dir_for_home(write_home);
    std::fs::create_dir_all(&commands_dir).with_context(|| {
        format!(
            "failed to create Claude commands dir at {}",
            commands_dir.display()
        )
    })?;

    let skill_specs = collect_skill_specs(write_home);
    let workflow_specs = collect_workflow_specs(write_home);

    let mut desired: BTreeMap<String, String> = BTreeMap::new();
    for (slug, description, trigger) in &skill_specs {
        let content = format!(
            "{description}\n\nUse the mcp__agent007__agent007_skill_run tool with trigger \"{trigger}\" and args \"$ARGUMENTS\".\n"
        );
        desired.insert(format!("agent007-{slug}.md"), content);
    }
    for (name, description) in &workflow_specs {
        let content = format!(
            "{}\n\nUse the mcp__agent007__agent007_workflow_run tool with name=\"{}\" and task=\"$ARGUMENTS\".\n",
            if description.is_empty() {
                format!("Run the {name} workflow")
            } else {
                description.to_string()
            },
            name,
        );
        desired.insert(format!("agent007-workflow-{name}.md"), content);
    }

    let mut written = 0usize;
    for (name, content) in &desired {
        let path = commands_dir.join(name);
        let existing = std::fs::read_to_string(&path).ok();
        if existing.as_deref() != Some(content.as_str()) {
            std::fs::write(&path, content)
                .with_context(|| format!("failed to write {}", path.display()))?;
            written += 1;
        }
    }

    let mut removed = 0usize;
    for entry in std::fs::read_dir(&commands_dir)?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".md") {
            continue;
        }
        if !name.starts_with("agent007-") {
            continue;
        }
        if desired.contains_key(name) {
            continue;
        }
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
        removed += 1;
    }

    Ok(SlashCommandSyncSummary {
        commands_dir,
        written,
        removed,
        skill_commands: skill_specs.len(),
        workflow_commands: workflow_specs.len(),
    })
}

fn collect_skill_specs(write_home: &Path) -> Vec<(String, String, String)> {
    let mut specs = Vec::new();
    let mut seen = HashSet::new();

    for skills_dir in asset_homes_for_sync(write_home)
        .into_iter()
        .flat_map(|home| {
            let mut dirs = vec![home.join("skills")];
            dirs.extend(agent007_core::paths::enabled_pack_asset_dirs(
                &home, "skills",
            ));
            dirs
        })
    {
        if !skills_dir.exists() {
            continue;
        }
        let loader = agent007_skills::SkillLoader::new(&skills_dir);
        let Ok(skills) = loader.load_all() else {
            continue;
        };
        for skill in skills {
            let trigger = normalize_trigger(skill.trigger());
            let key = trigger.to_ascii_lowercase();
            if !seen.insert(key) {
                continue;
            }
            specs.push((
                command_slug(&trigger),
                skill.frontmatter.description.clone(),
                trigger,
            ));
        }
    }

    specs
}

fn collect_workflow_specs(write_home: &Path) -> Vec<(String, String)> {
    let mut specs = Vec::new();
    let mut seen = HashSet::new();

    for workflows_dir in asset_homes_for_sync(write_home)
        .into_iter()
        .flat_map(|home| {
            let mut dirs = vec![home.join("workflows")];
            dirs.extend(agent007_core::paths::enabled_pack_asset_dirs(
                &home,
                "workflows",
            ));
            dirs
        })
    {
        if !workflows_dir.exists() {
            continue;
        }
        let loader = agent007_workflows::WorkflowLoader::new(workflows_dir);
        let Ok(names) = loader.list_names() else {
            continue;
        };
        for name in names {
            let key = name.to_ascii_lowercase();
            if !seen.insert(key) {
                continue;
            }
            if let Ok(def) = loader.load_named(&name) {
                specs.push((name, def.description.unwrap_or_default()));
            }
        }
    }

    specs
}

fn normalize_trigger(trigger: &str) -> String {
    let trimmed = trigger.trim();
    let bare = trimmed.trim_start_matches('/');
    if bare.is_empty() {
        return "/custom-skill".to_string();
    }
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn command_slug(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('/')
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

fn asset_homes_for_sync(write_home: &Path) -> Vec<PathBuf> {
    let mut homes = vec![write_home.to_path_buf()];
    let global = agent007_global_home();
    if !paths_equal(write_home, &global) {
        homes.push(global);
    }
    homes
}

fn claude_commands_dir_for_home(write_home: &Path) -> PathBuf {
    let global = agent007_global_home();
    if paths_equal(write_home, &global) {
        return home_dir().join(".claude").join("commands");
    }
    let project_root = write_home
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    project_root.join(".claude").join("commands")
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_lock;

    fn write_skill(home: &Path, file_name: &str, trigger: &str, description: &str) {
        let skills = home.join("skills");
        std::fs::create_dir_all(&skills).unwrap();
        std::fs::write(
            skills.join(format!("{file_name}.md")),
            format!(
                "---\nname: Test\ntrigger: {trigger}\ndescription: {description}\nmodel: codex\n---\nDo {{args}}\n"
            ),
        )
        .unwrap();
    }

    fn write_workflow(home: &Path, file_name: &str, description: &str) {
        let workflows = home.join("workflows");
        std::fs::create_dir_all(&workflows).unwrap();
        std::fs::write(
            workflows.join(format!("{file_name}.yaml")),
            format!(
                "name: {file_name}\ndescription: {description}\nsteps:\n  - id: one\n    agent: Coder\n    prompt: hi\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn sync_creates_claude_command_files_for_skills_and_workflows() {
        let _guard = env_lock();
        let project = tempfile::tempdir().unwrap();
        let previous_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", project.path());

        let home = project.path().join(".agent007");
        std::fs::create_dir_all(&home).unwrap();
        write_skill(&home, "my_skill", "/my-skill", "Run my skill");
        write_workflow(&home, "demo", "Demo workflow");

        let summary = sync_claude_slash_commands_for_home(&home).unwrap();
        assert_eq!(summary.skill_commands, 1);
        assert_eq!(summary.workflow_commands, 1);

        let commands = project.path().join(".claude").join("commands");
        assert!(commands.join("agent007-my-skill.md").exists());
        assert!(commands.join("agent007-workflow-demo.md").exists());

        if let Some(home) = previous_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
    }
}
