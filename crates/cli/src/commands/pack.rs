use crate::commands::slash_commands::sync_claude_slash_commands_for_home;
use agent007_packs::{
    build_pack_artifact, LockedPack, PackManager, RegistryPack, DEFAULT_REGISTRY_URL,
};
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct PackArgs {
    #[command(subcommand)]
    pub action: PackAction,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PackScopeArg {
    Global,
    Project,
}

#[derive(Subcommand, Debug)]
pub enum PackAction {
    /// Build a text-safe .a7bundle artifact from a pack source directory
    Build {
        source: PathBuf,
        #[arg(long, short = 'o')]
        output: PathBuf,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Verify every manifest, artifact, and entry in a registry
    VerifyRegistry {
        #[arg(long)]
        registry: Option<String>,
        #[arg(long, default_value_t = true)]
        refresh: bool,
        #[arg(long, default_value_t = false)]
        offline: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Search the official pack registry
    Search {
        #[arg(default_value = "")]
        query: String,
        #[arg(long)]
        registry: Option<String>,
        #[arg(long, default_value_t = false)]
        refresh: bool,
        #[arg(long, default_value_t = false)]
        offline: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Show registry metadata and available versions for a pack
    Info {
        id: String,
        #[arg(long)]
        registry: Option<String>,
        #[arg(long, default_value_t = false)]
        refresh: bool,
        #[arg(long, default_value_t = false)]
        offline: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// List installed packs in one scope
    List {
        #[arg(long, value_enum, default_value_t = PackScopeArg::Global)]
        scope: PackScopeArg,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Download, verify, install, and enable a pack
    Install {
        id: String,
        #[arg(long)]
        version: Option<String>,
        #[arg(long, value_enum, default_value_t = PackScopeArg::Global)]
        scope: PackScopeArg,
        #[arg(long)]
        registry: Option<String>,
        #[arg(long, default_value_t = false)]
        refresh: bool,
        #[arg(long, default_value_t = false)]
        offline: bool,
        #[arg(long, default_value_t = false)]
        disabled: bool,
        /// Approve installing a pack that declares consequential external actions
        #[arg(long, default_value_t = false)]
        allow_external_actions: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Enable an installed pack
    Enable {
        id: String,
        #[arg(long, value_enum, default_value_t = PackScopeArg::Global)]
        scope: PackScopeArg,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Disable an installed pack without deleting it
    Disable {
        id: String,
        #[arg(long, value_enum, default_value_t = PackScopeArg::Global)]
        scope: PackScopeArg,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Update an installed pack to the newest compatible version
    Update {
        id: String,
        #[arg(long, value_enum, default_value_t = PackScopeArg::Global)]
        scope: PackScopeArg,
        #[arg(long)]
        registry: Option<String>,
        /// Use the cached registry instead of refreshing before the update
        #[arg(long, default_value_t = false)]
        no_refresh: bool,
        #[arg(long, default_value_t = false)]
        offline: bool,
        /// Approve updating to a version that declares consequential external actions
        #[arg(long, default_value_t = false)]
        allow_external_actions: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Switch to the previously installed version
    Rollback {
        id: String,
        #[arg(long, value_enum, default_value_t = PackScopeArg::Global)]
        scope: PackScopeArg,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Remove a pack and all of its installed versions
    Uninstall {
        id: String,
        #[arg(long, value_enum, default_value_t = PackScopeArg::Global)]
        scope: PackScopeArg,
        /// Confirm destructive removal
        #[arg(long, default_value_t = false)]
        yes: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

pub async fn execute(action: PackAction) -> Result<()> {
    match action {
        PackAction::Build {
            source,
            output,
            json,
        } => {
            let artifact = build_pack_artifact(&source, &output)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&artifact)?);
            } else {
                println!("Built {}", artifact.path.display());
                println!("SHA-256: {}", artifact.sha256);
                println!("Size: {} bytes", artifact.size_bytes);
                println!(
                    "Contents: {} skill(s), {} workflow(s), {} persona(s), {} tool file(s)",
                    artifact.skills, artifact.workflows, artifact.personas, artifact.tools
                );
            }
        }
        PackAction::VerifyRegistry {
            registry,
            refresh,
            offline,
            json,
        } => {
            let report = registry_manager(registry, offline)?
                .verify_registry(refresh)
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "Registry verification: {} pack(s), {} version(s)",
                    report.packs_checked, report.versions_checked
                );
                for error in &report.errors {
                    eprintln!("  ERROR: {error}");
                }
                println!("Result: {}", if report.valid { "valid" } else { "invalid" });
            }
            if !report.valid {
                bail!("registry verification failed");
            }
        }
        PackAction::Search {
            query,
            registry,
            refresh,
            offline,
            json,
        } => {
            let manager = registry_manager(registry, offline)?;
            let packs = manager.search(&query, refresh).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&packs)?);
            } else if packs.is_empty() {
                println!("No packs matched '{}'.", query);
            } else {
                println!("Available packs:");
                for pack in packs {
                    let latest = latest_version(&pack).unwrap_or("unpublished");
                    println!("  {} {} — {}", pack.id, latest, pack.description);
                }
            }
        }
        PackAction::Info {
            id,
            registry,
            refresh,
            offline,
            json,
        } => {
            let manager = registry_manager(registry, offline)?;
            let inspection = manager.inspect(&id, None, refresh).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&inspection)?);
            } else {
                let pack = inspection.pack;
                println!("{} ({})", pack.name, pack.id);
                println!("{}", pack.description);
                println!("Categories: {}", pack.categories.join(", "));
                println!(
                    "Contents: {} skill(s), {} workflow(s), {} persona(s), {} tool(s)",
                    inspection.manifest.contents.skills.len(),
                    inspection.manifest.contents.workflows.len(),
                    inspection.manifest.contents.personas.len(),
                    inspection.manifest.contents.tools.len()
                );
                println!(
                    "Permissions: network={}, external_actions={}, approvals=[{}]",
                    inspection.manifest.permissions.network,
                    inspection.manifest.permissions.external_actions,
                    inspection.manifest.permissions.approval_required.join(", ")
                );
                println!("Versions:");
                let mut versions = pack.versions;
                versions.sort_by(|left, right| right.version.cmp(&left.version));
                for version in versions {
                    let yanked = if version.yanked { " [yanked]" } else { "" };
                    println!(
                        "  {}{} (requires agent007 >= {})",
                        version.version, yanked, version.min_agent007
                    );
                }
            }
        }
        PackAction::List { scope, json } => {
            let manager = scoped_manager(scope, None, false)?;
            let lock = manager.load_lock()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&lock)?);
            } else if lock.packs.is_empty() {
                println!("No packs installed in {}.", manager.home().display());
            } else {
                println!("Installed packs in {}:", manager.home().display());
                for pack in lock.packs.values() {
                    print_locked(pack);
                }
            }
        }
        PackAction::Install {
            id,
            version,
            scope,
            registry,
            refresh,
            offline,
            disabled,
            allow_external_actions,
            json,
        } => {
            let manager = scoped_manager(scope, registry, offline)?
                .with_external_actions_allowed(allow_external_actions);
            let result = manager
                .install(&id, version.as_deref(), !disabled, refresh)
                .await?;
            sync_after_change(&manager);
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "Installed {}@{} at {} ({})",
                    result.id,
                    result.version,
                    result.install_dir.display(),
                    if result.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
                if !result.dependencies_installed.is_empty() {
                    println!("Dependencies: {}", result.dependencies_installed.join(", "));
                }
            }
        }
        PackAction::Enable { id, scope, json } => {
            let manager = scoped_manager(scope, None, false)?;
            let pack = manager.enable(&id)?;
            sync_after_change(&manager);
            print_lifecycle(pack, json)?;
        }
        PackAction::Disable { id, scope, json } => {
            let manager = scoped_manager(scope, None, false)?;
            let pack = manager.disable(&id)?;
            sync_after_change(&manager);
            print_lifecycle(pack, json)?;
        }
        PackAction::Update {
            id,
            scope,
            registry,
            no_refresh,
            offline,
            allow_external_actions,
            json,
        } => {
            let manager = scoped_manager(scope, registry, offline)?
                .with_external_actions_allowed(allow_external_actions);
            let result = manager.update(&id, !no_refresh).await?;
            sync_after_change(&manager);
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("Updated {} to {}.", result.id, result.version);
            }
        }
        PackAction::Rollback { id, scope, json } => {
            let manager = scoped_manager(scope, None, false)?;
            let pack = manager.rollback(&id)?;
            sync_after_change(&manager);
            print_lifecycle(pack, json)?;
        }
        PackAction::Uninstall {
            id,
            scope,
            yes,
            json,
        } => {
            if !yes {
                bail!("refusing to uninstall '{id}' without --yes");
            }
            let manager = scoped_manager(scope, None, false)?;
            manager.uninstall(&id)?;
            sync_after_change(&manager);
            if json {
                println!("{}", serde_json::json!({"removed": id}));
            } else {
                println!("Uninstalled {id}.");
            }
        }
    }
    Ok(())
}

fn default_registry() -> String {
    std::env::var("AGENT007_PACK_REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY_URL.to_string())
}

fn registry_manager(registry: Option<String>, offline: bool) -> Result<PackManager> {
    scoped_manager(PackScopeArg::Global, registry, offline)
}

fn scoped_manager(
    scope: PackScopeArg,
    registry: Option<String>,
    offline: bool,
) -> Result<PackManager> {
    let home = scope_home(scope)?;
    PackManager::new(
        home,
        registry.unwrap_or_else(default_registry),
        env!("CARGO_PKG_VERSION"),
    )
    .map(|manager| manager.with_offline(offline))
}

fn scope_home(scope: PackScopeArg) -> Result<PathBuf> {
    if let Ok(home) = std::env::var("AGENT007_HOME") {
        return Ok(PathBuf::from(home));
    }
    match scope {
        PackScopeArg::Global => Ok(agent007_core::paths::agent007_global_home()),
        PackScopeArg::Project => {
            if let Some(home) = agent007_core::paths::agent007_project_home() {
                return Ok(home);
            }
            let cwd = std::env::current_dir().context("resolve current project directory")?;
            Ok(cwd.join(".agent007"))
        }
    }
}

fn latest_version(pack: &RegistryPack) -> Option<&str> {
    pack.versions
        .iter()
        .filter(|version| !version.yanked)
        .max_by(|left, right| {
            match (
                semver::Version::parse(&left.version),
                semver::Version::parse(&right.version),
            ) {
                (Ok(left), Ok(right)) => left.cmp(&right),
                _ => left.version.cmp(&right.version),
            }
        })
        .map(|version| version.version.as_str())
}

fn sync_after_change(manager: &PackManager) {
    if let Err(error) = sync_claude_slash_commands_for_home(manager.home()) {
        eprintln!("warning: pack state changed, but Claude slash-command sync failed: {error}");
    }
}

fn print_locked(pack: &LockedPack) {
    println!(
        "  {} {} [{}]",
        pack.id,
        pack.version,
        if pack.enabled { "enabled" } else { "disabled" }
    );
}

fn print_lifecycle(pack: LockedPack, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        print_locked(&pack);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_refreshes_by_default_and_supports_offline_cache() {
        let args = PackArgs::try_parse_from(["pack", "update", "example", "--offline"])
            .expect("update args should parse");
        match args.action {
            PackAction::Update {
                no_refresh,
                offline,
                ..
            } => {
                assert!(!no_refresh);
                assert!(offline);
            }
            _ => panic!("wrong subcommand"),
        }
    }

    #[test]
    fn external_actions_require_an_explicit_cli_flag() {
        let args =
            PackArgs::try_parse_from(["pack", "install", "example", "--allow-external-actions"])
                .expect("install args should parse");
        match args.action {
            PackAction::Install {
                allow_external_actions,
                ..
            } => assert!(allow_external_actions),
            _ => panic!("wrong subcommand"),
        }
    }
}
