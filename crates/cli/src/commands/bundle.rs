use crate::commands::slash_commands::sync_claude_slash_commands_for_home;
use crate::config::Config;
use crate::BundleAction;
use agent007_core::paths::{agent007_global_home, agent007_write_home, skills_search_dirs, workflow_search_dirs};
use agent007_sharing::{Bundle, BundleBuilder, BundleImporter};
use anyhow::Result;
use std::sync::Arc;

pub async fn execute(_config: Arc<Config>, action: BundleAction) -> Result<()> {
    match action {
        BundleAction::Export {
            output,
            skills,
            workflows,
        } => {
            let builder = BundleBuilder::new(skills_search_dirs(), workflow_search_dirs());

            let skill_refs: Vec<&str> = skills.iter().map(String::as_str).collect();
            let wf_refs: Vec<&str> = workflows.iter().map(String::as_str).collect();
            let bundle = builder.build(&skill_refs, &wf_refs)?;

            let json = bundle.to_json()?;
            let dest = output.unwrap_or_else(|| "agent007-bundle.a7bundle".to_string());
            std::fs::write(&dest, &json)?;
            println!(
                "✓ Exported {} skill(s), {} workflow(s), and {} tool file(s) to {dest}",
                bundle.skills.len(),
                bundle.workflows.len(),
                bundle.tools.len()
            );
            Ok(())
        }

        BundleAction::Import {
            file,
            overwrite,
            global,
        } => {
            let content = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("cannot read {file}: {e}"))?;
            let bundle =
                Bundle::from_json(&content).map_err(|e| anyhow::anyhow!("invalid bundle: {e}"))?;

            println!(
                "Bundle contains {} skill(s), {} workflow(s), and {} tool file(s)",
                bundle.skills.len(),
                bundle.workflows.len(),
                bundle.tools.len()
            );

            let target = if global {
                agent007_global_home()
            } else {
                agent007_write_home()
            };
            let skills_dir = target.join("skills");
            let workflows_dir = target.join("workflows");
            let importer = BundleImporter::new(&skills_dir, &workflows_dir);
            let results = importer.import(&bundle, overwrite)?;

            let imported: Vec<_> = results
                .iter()
                .filter(|r| r.action == agent007_sharing::ImportAction::Imported)
                .collect();
            let skipped: Vec<_> = results
                .iter()
                .filter(|r| r.action == agent007_sharing::ImportAction::Skipped)
                .collect();
            let overwritten: Vec<_> = results
                .iter()
                .filter(|r| r.action == agent007_sharing::ImportAction::Overwritten)
                .collect();

            for r in &imported {
                println!("  + {}", r.filename);
            }
            for r in &overwritten {
                println!("  ↺ {} (overwritten)", r.filename);
            }
            for r in &skipped {
                println!("  ⊘ {} (skipped — use --overwrite)", r.filename);
            }

            println!(
                "\n✓ Done: {} imported, {} skipped, {} overwritten",
                imported.len(),
                skipped.len(),
                overwritten.len()
            );
            match sync_claude_slash_commands_for_home(&target) {
                Ok(summary) => {
                    println!(
                        "✓ Synced Claude slash commands ({} skill, {} workflow) at {}",
                        summary.skill_commands,
                        summary.workflow_commands,
                        summary.commands_dir.display()
                    );
                }
                Err(error) => {
                    eprintln!(
                        "⚠ Imported bundle but could not sync Claude slash commands: {error}"
                    );
                }
            }
            Ok(())
        }

        BundleAction::Promote {
            skill,
            workflow,
            global,
        } => {
            let src_home = agent007_write_home();
            let target_home = if global {
                agent007_global_home()
            } else {
                agent007_global_home()
            };

            if let Some(trigger) = skill {
                promote_skill(&src_home, &target_home, &trigger)?;
            }
            if let Some(name) = workflow {
                promote_workflow(&src_home, &target_home, &name)?;
            }
            match sync_claude_slash_commands_for_home(&target_home) {
                Ok(summary) => {
                    println!(
                        "✓ Synced Claude slash commands ({} skill, {} workflow) at {}",
                        summary.skill_commands,
                        summary.workflow_commands,
                        summary.commands_dir.display()
                    );
                }
                Err(error) => {
                    eprintln!(
                        "⚠ Promotion completed but could not sync Claude slash commands: {error}"
                    );
                }
            }
            Ok(())
        }
    }
}

fn promote_skill(
    src_home: &std::path::Path,
    target_home: &std::path::Path,
    trigger: &str,
) -> Result<()> {
    let normalized = format!("/{}", trigger.trim_start_matches('/'));
    let skills_src = src_home.join("skills");
    let skills_dest = target_home.join("skills");

    let found = std::fs::read_dir(&skills_src)?
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
        .find(|e| {
            std::fs::read_to_string(e.path())
                .ok()
                .and_then(|c| parse_trigger(&c))
                .map(|t| t == normalized)
                .unwrap_or(false)
        });

    let entry = found.ok_or_else(|| anyhow::anyhow!("skill with trigger '{trigger}' not found"))?;
    let filename = entry
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    std::fs::create_dir_all(&skills_dest)?;
    let dest = skills_dest.join(&filename);
    if dest.exists() {
        anyhow::bail!(
            "skill already exists globally at {}: use a different approach or remove it first",
            dest.display()
        );
    }
    std::fs::copy(entry.path(), &dest)?;
    println!("✓ Skill '{trigger}' promoted to {}", dest.display());
    Ok(())
}

fn promote_workflow(
    src_home: &std::path::Path,
    target_home: &std::path::Path,
    name: &str,
) -> Result<()> {
    let src_dir = src_home.join("workflows");
    let dest_dir = target_home.join("workflows");

    let src = [
        src_dir.join(format!("{name}.yaml")),
        src_dir.join(format!("{name}.yml")),
    ]
    .into_iter()
    .find(|p| p.exists())
    .ok_or_else(|| anyhow::anyhow!("workflow '{name}' not found in project"))?;

    std::fs::create_dir_all(&dest_dir)?;
    let filename = src.file_name().unwrap().to_string_lossy().to_string();
    let dest = dest_dir.join(&filename);
    if dest.exists() {
        anyhow::bail!("workflow already exists globally at {}", dest.display());
    }
    std::fs::copy(&src, &dest)?;
    println!("✓ Workflow '{name}' promoted to {}", dest.display());
    Ok(())
}

fn parse_trigger(content: &str) -> Option<String> {
    let body = content.strip_prefix("---")?;
    let end = body.find("---")?;
    let yaml = &body[..end];
    for line in yaml.lines() {
        if let Some(rest) = line.strip_prefix("trigger:") {
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}
