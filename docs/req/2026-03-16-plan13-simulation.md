# Simulation Agent Pipeline Crate Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `agent007-simulation` crate providing a 5-stage simulation pipeline (Researcher → ScenarioGen → Simulator → Validator → Reporter) with TOML-defined simulation templates, two built-in WiFi templates (RTT + Roaming), user-defined custom templates, and `agent007 simulate` CLI commands.

**Architecture:** New `crates/simulation` crate. `SimulationTemplate` parsed from TOML. Each pipeline stage is a struct. `SimulationPipeline` runs them in sequence. `Simulator` writes scenario inputs to a temp file, invokes the `[system_under_test]` command, reads outputs. `Reporter` stores structured results in memory for regression detection. Built-in templates ship in `crates/simulation/templates/`.

**Tech Stack:** Rust, thiserror, serde/toml, serde_json, tokio, tera (workspace), agent007-core, agent007-memory, agent007-models

**Prerequisites:** Plans 1–4 complete. All library crates built and tested.

**Spec:** `docs/superpowers/specs/2026-03-16-agent007-design.md`

---

## File Structure

```
crates/simulation/
├── Cargo.toml
├── templates/
│   ├── wifi-rtt.toml
│   └── wifi-roaming.toml
└── src/
    ├── lib.rs          # pub re-exports: SimulationError, SimulationPipeline, TemplateLoader, SimulationTemplate, SimulationReport
    ├── error.rs        # SimulationError (thiserror)
    ├── types.rs        # SimulationTemplate, SystemUnderTest, ScenarioDef, ValidationConfig, OutputConfig,
    │                   #   SimulationReport, ScenarioFailure
    ├── loader.rs       # TemplateLoader — built-in + custom template resolution
    ├── simulator.rs    # Simulator — SUT invocation, temp-file I/O, timeout handling
    └── pipeline.rs     # SimulationPipeline — all 5 stages

crates/cli/src/commands/
└── simulate.rs         # Replace existing stub with full implementation

Modified files:
    Cargo.toml                              (root workspace — add crates/simulation to members)
    crates/cli/Cargo.toml                   (add agent007-simulation dependency)
```

---

## Chunk 1: Scaffold crate + error type

### Task 1: Add simulation crate to workspace; create Cargo.toml and error type

**Files:**
- Create: `crates/simulation/Cargo.toml`
- Create: `crates/simulation/src/lib.rs`
- Create: `crates/simulation/src/error.rs`
- Modify: `Cargo.toml` (root — add `"crates/simulation"` to `members`)

**Step 1 — write a failing test**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn error_variants_exist() {
        // Will fail to compile until SimulationError is defined
        use crate::SimulationError;
        let _e = SimulationError::TemplateNotFound { name: "wifi-rtt".into() };
    }
}
```

Run — expect compile failure:
```bash
cargo test -p agent007-simulation 2>&1 | head -20
```

**Step 2 — implement**

`crates/simulation/Cargo.toml`:
```toml
[package]
name = "agent007-simulation"
version = "0.1.0"
edition = "2021"

[dependencies]
agent007-core   = { path = "../core" }
agent007-memory = { path = "../memory" }
agent007-models = { path = "../models" }
thiserror   = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
toml        = { workspace = true }
tera        = { workspace = true }
tokio       = { workspace = true }
uuid        = { workspace = true }
chrono      = { workspace = true }
tracing     = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

`crates/simulation/src/error.rs`:
```rust
use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum SimulationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse template {path}: {reason}")]
    ParseError { path: PathBuf, reason: String },
    #[error("template '{name}' not found")]
    TemplateNotFound { name: String },
    #[error("system under test failed (exit code {code}): {stderr}")]
    SutFailed { code: i32, stderr: String },
    #[error("scenario '{name}' timed out after {secs}s")]
    Timeout { name: String, secs: u64 },
    #[error("validation failed for scenario '{name}': {reason}")]
    ValidationFailed { name: String, reason: String },
    #[error("model error: {0}")]
    ModelError(String),
}
```

`crates/simulation/src/lib.rs`:
```rust
pub mod error;
pub mod types;
pub mod loader;
pub mod simulator;
pub mod pipeline;

pub use error::SimulationError;
pub use types::{
    OutputConfig, ScenarioDef, ScenarioFailure, SimulationReport, SimulationTemplate,
    SystemUnderTest, ValidationConfig,
};
pub use loader::TemplateLoader;
pub use pipeline::SimulationPipeline;
```

Root `Cargo.toml` — add to `members`:
```toml
members = [
    # ... existing entries ...
    "crates/simulation",
]
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-simulation
```

- [ ] Task 1 complete

---

## Chunk 2: Core types

### Task 2: Implement SimulationTemplate and SimulationReport structs

**Files:**
- Create: `crates/simulation/src/types.rs`

**Step 1 — write failing tests**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn report_roundtrip() {
        use crate::types::SimulationReport;
        let r = SimulationReport {
            run_id: "r1".into(),
            template_name: "wifi-rtt".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            scenarios_run: 5,
            scenarios_passed: 5,
            scenarios_failed: 0,
            failures: vec![],
            regressions: vec![],
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: SimulationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.run_id, "r1");
    }
}
```

Run — expect compile failure:
```bash
cargo test -p agent007-simulation 2>&1 | head -20
```

**Step 2 — implement**

`crates/simulation/src/types.rs`:
```rust
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Template definition — parsed from TOML files
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug, Clone)]
pub struct SimulationTemplate {
    pub name: String,
    pub description: Option<String>,
    pub research_topics: Vec<String>,
    pub system_under_test: SystemUnderTest,
    pub scenarios: Vec<ScenarioDef>,
    #[serde(default)]
    pub validation: ValidationConfig,
    #[serde(default)]
    pub output: OutputConfig,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SystemUnderTest {
    /// Binary/command to invoke (e.g. "cargo", "./my_sut").
    pub command: String,
    /// Arguments; may contain `{{scenario_file}}` and `{{output_file}}` placeholders.
    #[serde(default)]
    pub args: Vec<String>,
    /// Maximum wall-clock seconds before the process is killed.
    pub timeout_secs: Option<u64>,
    /// Working directory for the subprocess (defaults to cwd).
    pub working_dir: Option<String>,
    /// Extra environment variables.
    pub env: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ScenarioDef {
    pub name: String,
    /// All other fields are domain-specific parameters.
    #[serde(flatten)]
    pub params: serde_json::Value,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct ValidationConfig {
    /// Maximum positioning error in metres (WiFi RTT).
    pub max_error_m: Option<f64>,
    /// Minimum accuracy percentage.
    pub min_accuracy_percent: Option<f64>,
    /// Maximum handoff time in milliseconds (WiFi roaming).
    pub max_handoff_time_ms: Option<u64>,
    /// Minimum successful connection rate (0.0–1.0).
    pub min_connection_success_rate: Option<f64>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct OutputConfig {
    /// Output format: "csv" or "json".
    pub format: Option<String>,
    /// Persist the SimulationReport to the memory store.
    pub store_in_memory: Option<bool>,
    /// Compare this run's results against the previous stored report.
    pub compare_with_previous: Option<bool>,
}

// ---------------------------------------------------------------------------
// Run results
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SimulationReport {
    pub run_id: String,
    pub template_name: String,
    pub timestamp: String,
    pub scenarios_run: usize,
    pub scenarios_passed: usize,
    pub scenarios_failed: usize,
    pub failures: Vec<ScenarioFailure>,
    pub regressions: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ScenarioFailure {
    pub scenario: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_roundtrip() {
        let r = SimulationReport {
            run_id: "r1".into(),
            template_name: "wifi-rtt".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            scenarios_run: 5,
            scenarios_passed: 5,
            scenarios_failed: 0,
            failures: vec![],
            regressions: vec![],
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: SimulationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.run_id, "r1");
        assert_eq!(back.template_name, "wifi-rtt");
    }

    #[test]
    fn validation_config_defaults() {
        let v = ValidationConfig::default();
        assert!(v.max_error_m.is_none());
        assert!(v.min_accuracy_percent.is_none());
    }

    #[test]
    fn template_parses_from_toml() {
        let toml_str = r#"
name = "smoke-test"
description = "minimal test"
research_topics = []

[system_under_test]
command = "echo"
args = ["hello"]

[[scenarios]]
name = "s1"
foo = "bar"
        "#;
        let t: SimulationTemplate = toml::from_str(toml_str).unwrap();
        assert_eq!(t.name, "smoke-test");
        assert_eq!(t.scenarios.len(), 1);
        assert_eq!(t.scenarios[0].name, "s1");
    }
}
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-simulation
```

- [ ] Task 2 complete

---

## Chunk 3: Built-in templates

### Task 3: Create wifi-rtt.toml and wifi-roaming.toml; embed via include_str!

**Files:**
- Create: `crates/simulation/templates/wifi-rtt.toml`
- Create: `crates/simulation/templates/wifi-roaming.toml`

**Step 1 — write failing test**

```rust
#[test]
fn builtin_templates_parse() {
    // Will fail until the TOML files exist and are valid
    use crate::types::SimulationTemplate;
    let rtt: SimulationTemplate =
        toml::from_str(include_str!("../templates/wifi-rtt.toml")).unwrap();
    assert_eq!(rtt.name, "wifi-rtt");
    let roaming: SimulationTemplate =
        toml::from_str(include_str!("../templates/wifi-roaming.toml")).unwrap();
    assert_eq!(roaming.name, "wifi-roaming");
}
```

Add to `crates/simulation/src/types.rs` tests, then run:
```bash
cargo test -p agent007-simulation 2>&1 | head -20
```

**Step 2 — implement**

`crates/simulation/templates/wifi-rtt.toml`:
```toml
name = "wifi-rtt"
description = "WiFi Round-Trip Time (RTT) / Fine Timing Measurement positioning simulation"
research_topics = [
    "IEEE 802.11mc FTM protocol",
    "path loss models (free-space, log-distance)",
    "multipath propagation effects",
    "anchor placement algorithms",
]

[system_under_test]
command = "{{sut_command}}"
args    = ["--scenario", "{{scenario_file}}", "--output", "{{output_file}}"]
timeout_secs  = 30
working_dir   = "."

[validation]
max_error_m            = 2.0
min_accuracy_percent   = 90.0

[output]
format               = "json"
store_in_memory      = true
compare_with_previous = true

[[scenarios]]
name              = "open_office_3ap"
description       = "Open-plan office, 3 access points, 10 measurement points"
room_width_m      = 20.0
room_depth_m      = 15.0
anchor_positions  = [[2.0, 2.0], [10.0, 2.0], [18.0, 2.0]]
measurement_grid  = 1.0
noise_std_ns      = 5.0

[[scenarios]]
name              = "corridor_2ap"
description       = "Narrow corridor, 2 access points at opposite ends"
room_width_m      = 30.0
room_depth_m      = 3.0
anchor_positions  = [[1.0, 1.5], [29.0, 1.5]]
measurement_grid  = 1.0
noise_std_ns      = 8.0

[[scenarios]]
name              = "multipath_heavy"
description       = "Conference room with high multipath (glass walls)"
room_width_m      = 10.0
room_depth_m      = 8.0
anchor_positions  = [[1.0, 1.0], [9.0, 1.0], [5.0, 7.0]]
measurement_grid  = 0.5
noise_std_ns      = 20.0
multipath_factor  = 3.0
```

`crates/simulation/templates/wifi-roaming.toml`:
```toml
name = "wifi-roaming"
description = "WiFi roaming / handoff simulation between access points"
research_topics = [
    "IEEE 802.11r Fast BSS Transition",
    "RSSI-based roaming triggers",
    "handoff latency measurement",
    "seamless connectivity during mobility",
]

[system_under_test]
command = "{{sut_command}}"
args    = ["--scenario", "{{scenario_file}}", "--output", "{{output_file}}"]
timeout_secs  = 60
working_dir   = "."

[validation]
max_handoff_time_ms            = 150
min_connection_success_rate    = 0.99

[output]
format               = "json"
store_in_memory      = true
compare_with_previous = true

[[scenarios]]
name                    = "slow_walk_office"
description             = "Slow pedestrian walk across 3-AP office floor"
speed_mps               = 1.2
path_length_m           = 40.0
ap_positions            = [[5.0, 0.0], [20.0, 0.0], [35.0, 0.0]]
rssi_roam_threshold_dbm = -70
packet_loss_model       = "none"

[[scenarios]]
name                    = "fast_vehicle_highway"
description             = "Vehicle at highway speed, macro-cell roaming"
speed_mps               = 30.0
path_length_m           = 500.0
ap_positions            = [[50.0, 0.0], [200.0, 0.0], [400.0, 0.0]]
rssi_roam_threshold_dbm = -75
packet_loss_model       = "rayleigh"

[[scenarios]]
name                    = "elevator_vertical"
description             = "Elevator moving vertically between floors"
speed_mps               = 2.0
path_length_m           = 15.0
ap_positions            = [[0.0, 0.0], [0.0, 5.0], [0.0, 10.0], [0.0, 15.0]]
rssi_roam_threshold_dbm = -65
packet_loss_model       = "none"
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-simulation
```

- [ ] Task 3 complete

---

## Chunk 4: TemplateLoader

### Task 4: Implement TemplateLoader — built-in + custom template resolution

**Files:**
- Create: `crates/simulation/src/loader.rs`

**Step 1 — write failing tests**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn loader_finds_builtin_wifi_rtt() {
        // Will fail until TemplateLoader is defined
        use crate::loader::TemplateLoader;
        let loader = TemplateLoader::new_builtin_only();
        let t = loader.load("wifi-rtt").unwrap();
        assert_eq!(t.name, "wifi-rtt");
    }

    #[test]
    fn loader_lists_builtins() {
        use crate::loader::TemplateLoader;
        let loader = TemplateLoader::new_builtin_only();
        let names = loader.list();
        assert!(names.contains(&"wifi-rtt".to_string()));
        assert!(names.contains(&"wifi-roaming".to_string()));
    }
}
```

Run — expect compile failure:
```bash
cargo test -p agent007-simulation 2>&1 | head -20
```

**Step 2 — implement**

`crates/simulation/src/loader.rs`:
```rust
use std::path::PathBuf;

use crate::error::SimulationError;
use crate::types::SimulationTemplate;

// Embedded built-in templates (compile-time)
const BUILTIN_WIFI_RTT: &str =
    include_str!("../templates/wifi-rtt.toml");
const BUILTIN_WIFI_ROAMING: &str =
    include_str!("../templates/wifi-roaming.toml");

/// Maps built-in template names to their embedded TOML content.
fn builtin_templates() -> Vec<(&'static str, &'static str)> {
    vec![
        ("wifi-rtt", BUILTIN_WIFI_RTT),
        ("wifi-roaming", BUILTIN_WIFI_ROAMING),
    ]
}

/// Resolves simulation templates from built-in embedded strings or from the
/// filesystem (custom templates stored under `custom_dir`).
pub struct TemplateLoader {
    /// Optional override directory for built-in templates (useful in tests).
    pub builtin_dir: Option<PathBuf>,
    /// Directory for user-provided custom templates.
    /// Defaults to `~/.agent007/simulations/custom/` when `None`.
    pub custom_dir: Option<PathBuf>,
}

impl TemplateLoader {
    /// Create a loader that only uses embedded built-in templates.
    pub fn new_builtin_only() -> Self {
        Self {
            builtin_dir: None,
            custom_dir: None,
        }
    }

    /// Create a loader with a custom template directory.
    pub fn with_custom_dir(custom_dir: PathBuf) -> Self {
        Self {
            builtin_dir: None,
            custom_dir: Some(custom_dir),
        }
    }

    /// Load a template by name.
    ///
    /// Resolution order:
    /// 1. Built-in embedded templates (case-insensitive name match).
    /// 2. `custom/` prefix → look up in `custom_dir` (or `~/.agent007/simulations/custom/`).
    /// 3. Plain name → try `custom_dir` directly.
    pub fn load(&self, name: &str) -> Result<SimulationTemplate, SimulationError> {
        // 1. Built-in
        for (builtin_name, content) in builtin_templates() {
            if builtin_name.eq_ignore_ascii_case(name) {
                return toml::from_str(content).map_err(|e| SimulationError::ParseError {
                    path: PathBuf::from(format!("<builtin:{builtin_name}>")),
                    reason: e.to_string(),
                });
            }
        }

        // 2. Custom prefix
        let lookup_name = name.strip_prefix("custom/").unwrap_or(name);
        let custom_dir = self
            .custom_dir
            .clone()
            .unwrap_or_else(default_custom_dir);

        let candidates = [
            custom_dir.join(format!("{lookup_name}.toml")),
            custom_dir.join(lookup_name),
        ];

        for path in &candidates {
            if path.exists() {
                let content = std::fs::read_to_string(path)
                    .map_err(|e| SimulationError::ParseError {
                        path: path.clone(),
                        reason: e.to_string(),
                    })?;
                return toml::from_str(&content).map_err(|e| SimulationError::ParseError {
                    path: path.clone(),
                    reason: e.to_string(),
                });
            }
        }

        Err(SimulationError::TemplateNotFound { name: name.to_string() })
    }

    /// List all available template names.
    ///
    /// Returns built-in names first, then `custom/<filename_stem>` for every
    /// `.toml` file found in `custom_dir`.
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = builtin_templates()
            .into_iter()
            .map(|(n, _)| n.to_string())
            .collect();

        let custom_dir = self
            .custom_dir
            .clone()
            .unwrap_or_else(default_custom_dir);

        if let Ok(entries) = std::fs::read_dir(&custom_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        names.push(format!("custom/{stem}"));
                    }
                }
            }
        }

        names
    }
}

fn default_custom_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".agent007")
        .join("simulations")
        .join("custom")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn loader_finds_builtin_wifi_rtt() {
        let loader = TemplateLoader::new_builtin_only();
        let t = loader.load("wifi-rtt").unwrap();
        assert_eq!(t.name, "wifi-rtt");
        assert!(!t.scenarios.is_empty());
    }

    #[test]
    fn loader_finds_builtin_wifi_roaming() {
        let loader = TemplateLoader::new_builtin_only();
        let t = loader.load("wifi-roaming").unwrap();
        assert_eq!(t.name, "wifi-roaming");
    }

    #[test]
    fn loader_lists_builtins() {
        let loader = TemplateLoader::new_builtin_only();
        let names = loader.list();
        assert!(names.contains(&"wifi-rtt".to_string()));
        assert!(names.contains(&"wifi-roaming".to_string()));
    }

    #[test]
    fn loader_returns_not_found_for_unknown() {
        let loader = TemplateLoader::new_builtin_only();
        let err = loader.load("does-not-exist").unwrap_err();
        assert!(matches!(err, SimulationError::TemplateNotFound { .. }));
    }

    #[test]
    fn loader_finds_custom_template() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("gps-urban.toml"),
            r#"
name = "gps-urban"
description = "GPS urban canyon test"
research_topics = []
[system_under_test]
command = "echo"
[[scenarios]]
name = "downtown"
"#,
        )
        .unwrap();

        let loader = TemplateLoader::with_custom_dir(tmp.path().to_path_buf());
        let t = loader.load("custom/gps-urban").unwrap();
        assert_eq!(t.name, "gps-urban");
    }

    #[test]
    fn loader_lists_custom_templates() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("my-sim.toml"), "[system_under_test]\ncommand=\"echo\"\nname=\"my-sim\"\nresearch_topics=[]").unwrap();

        let loader = TemplateLoader::with_custom_dir(tmp.path().to_path_buf());
        let names = loader.list();
        assert!(names.contains(&"custom/my-sim".to_string()));
    }
}
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-simulation
```

- [ ] Task 4 complete

---

## Chunk 5: Simulator

### Task 5: Implement Simulator — SUT invocation, temp-file I/O, timeout handling

**Files:**
- Create: `crates/simulation/src/simulator.rs`

**Step 1 — write failing tests**

```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn simulator_runs_echo_sut() {
        // Will fail until Simulator is defined
        use crate::simulator::Simulator;
        use crate::types::{ScenarioDef, SystemUnderTest};
        use serde_json::json;

        let sut = SystemUnderTest {
            command: "echo".into(),
            args: vec!["hello".into()],
            timeout_secs: Some(5),
            working_dir: None,
            env: None,
        };
        let scenario = ScenarioDef {
            name: "s1".into(),
            params: json!({"x": 1}),
        };
        let sim = Simulator::new();
        let output = sim.run_scenario(&sut, &scenario).await.unwrap();
        assert!(!output.is_empty());
    }
}
```

Run — expect compile failure:
```bash
cargo test -p agent007-simulation 2>&1 | head -20
```

**Step 2 — implement**

`crates/simulation/src/simulator.rs`:
```rust
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

use crate::error::SimulationError;
use crate::types::{ScenarioDef, SystemUnderTest, ValidationConfig};

/// Invokes the system-under-test binary for a single scenario.
pub struct Simulator;

impl Simulator {
    pub fn new() -> Self {
        Self
    }

    /// Run one scenario and return the raw stdout output.
    ///
    /// The `{{scenario_file}}` placeholder in `sut.args` is replaced with the
    /// path to a temp file containing the scenario parameters as JSON.
    /// The `{{output_file}}` placeholder is replaced with a separate temp file
    /// path; its contents are returned as the output string.
    pub async fn run_scenario(
        &self,
        sut: &SystemUnderTest,
        scenario: &ScenarioDef,
    ) -> Result<String, SimulationError> {
        // Write scenario params to a temp file
        let scenario_tmp = tempfile::NamedTempFile::new()?;
        let output_tmp = tempfile::NamedTempFile::new()?;
        let scenario_path = scenario_tmp.path().to_string_lossy().to_string();
        let output_path = output_tmp.path().to_string_lossy().to_string();

        let scenario_json = serde_json::to_string_pretty(&scenario.params)
            .unwrap_or_else(|_| "{}".to_string());
        tokio::fs::write(&scenario_tmp.path(), scenario_json.as_bytes()).await?;

        // Expand placeholders in args
        let args: Vec<String> = sut
            .args
            .iter()
            .map(|a| {
                a.replace("{{scenario_file}}", &scenario_path)
                    .replace("{{output_file}}", &output_path)
            })
            .collect();

        // Build command
        let mut cmd = Command::new(&sut.command);
        cmd.args(&args);

        if let Some(wd) = &sut.working_dir {
            cmd.current_dir(wd);
        }

        if let Some(env) = &sut.env {
            for (k, v) in env {
                cmd.env(k, v);
            }
        }

        let timeout = Duration::from_secs(sut.timeout_secs.unwrap_or(30));

        let result = tokio::time::timeout(timeout, cmd.output()).await;

        match result {
            Err(_) => Err(SimulationError::Timeout {
                name: scenario.name.clone(),
                secs: sut.timeout_secs.unwrap_or(30),
            }),
            Ok(Err(e)) => Err(SimulationError::Io(e)),
            Ok(Ok(output)) => {
                if !output.status.success() {
                    let code = output.status.code().unwrap_or(-1);
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    return Err(SimulationError::SutFailed { code, stderr });
                }

                // Prefer {{output_file}} content; fall back to stdout
                let output_file_content =
                    tokio::fs::read_to_string(&output_tmp.path()).await.ok();

                if let Some(content) = output_file_content {
                    if !content.trim().is_empty() {
                        return Ok(content);
                    }
                }

                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            }
        }
    }

    /// Validate the raw output string against the template's ValidationConfig.
    ///
    /// For JSON output the validator attempts to parse and check known fields.
    /// Returns `Ok(())` if valid or if no validation constraints are set.
    pub fn validate_output(
        &self,
        scenario_name: &str,
        output: &str,
        config: &ValidationConfig,
    ) -> Result<(), SimulationError> {
        // Try to parse as JSON for structured validation
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(output) {
            if let Some(max_err) = config.max_error_m {
                if let Some(err_m) = v["error_m"].as_f64() {
                    if err_m > max_err {
                        return Err(SimulationError::ValidationFailed {
                            name: scenario_name.to_string(),
                            reason: format!(
                                "error_m {err_m:.2} exceeds max {max_err:.2}"
                            ),
                        });
                    }
                }
            }

            if let Some(min_acc) = config.min_accuracy_percent {
                if let Some(acc) = v["accuracy_percent"].as_f64() {
                    if acc < min_acc {
                        return Err(SimulationError::ValidationFailed {
                            name: scenario_name.to_string(),
                            reason: format!(
                                "accuracy_percent {acc:.1} below minimum {min_acc:.1}"
                            ),
                        });
                    }
                }
            }

            if let Some(max_ho) = config.max_handoff_time_ms {
                if let Some(ho) = v["handoff_time_ms"].as_u64() {
                    if ho > max_ho {
                        return Err(SimulationError::ValidationFailed {
                            name: scenario_name.to_string(),
                            reason: format!(
                                "handoff_time_ms {ho} exceeds max {max_ho}"
                            ),
                        });
                    }
                }
            }

            if let Some(min_csr) = config.min_connection_success_rate {
                if let Some(csr) = v["connection_success_rate"].as_f64() {
                    if csr < min_csr {
                        return Err(SimulationError::ValidationFailed {
                            name: scenario_name.to_string(),
                            reason: format!(
                                "connection_success_rate {csr:.3} below minimum {min_csr:.3}"
                            ),
                        });
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for Simulator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ScenarioDef;
    use serde_json::json;

    fn echo_sut() -> SystemUnderTest {
        SystemUnderTest {
            command: "echo".into(),
            args: vec!["hello".into()],
            timeout_secs: Some(5),
            working_dir: None,
            env: None,
        }
    }

    #[tokio::test]
    async fn simulator_runs_echo_sut() {
        let sut = echo_sut();
        let scenario = ScenarioDef {
            name: "s1".into(),
            params: json!({"x": 1}),
        };
        let sim = Simulator::new();
        let output = sim.run_scenario(&sut, &scenario).await.unwrap();
        assert!(!output.is_empty());
    }

    #[tokio::test]
    async fn simulator_detects_nonzero_exit() {
        let sut = SystemUnderTest {
            command: "false".into(),
            args: vec![],
            timeout_secs: Some(5),
            working_dir: None,
            env: None,
        };
        let scenario = ScenarioDef { name: "s1".into(), params: json!({}) };
        let sim = Simulator::new();
        let err = sim.run_scenario(&sut, &scenario).await.unwrap_err();
        assert!(matches!(err, SimulationError::SutFailed { .. }));
    }

    #[test]
    fn validate_passes_no_constraints() {
        let sim = Simulator::new();
        let config = ValidationConfig::default();
        assert!(sim.validate_output("s1", r#"{"anything": true}"#, &config).is_ok());
    }

    #[test]
    fn validate_fails_on_high_error_m() {
        let sim = Simulator::new();
        let config = ValidationConfig { max_error_m: Some(2.0), ..Default::default() };
        let err = sim
            .validate_output("s1", r#"{"error_m": 5.5}"#, &config)
            .unwrap_err();
        assert!(matches!(err, SimulationError::ValidationFailed { .. }));
    }

    #[test]
    fn validate_passes_within_bounds() {
        let sim = Simulator::new();
        let config = ValidationConfig { max_error_m: Some(2.0), ..Default::default() };
        assert!(sim
            .validate_output("s1", r#"{"error_m": 1.5}"#, &config)
            .is_ok());
    }
}
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-simulation
```

- [ ] Task 5 complete

---

## Chunk 6: SimulationPipeline — all 5 stages

### Task 6: Implement SimulationPipeline with all stages

**Files:**
- Create: `crates/simulation/src/pipeline.rs`

**Step 1 — write failing test**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn pipeline_stage_names_are_ordered() {
        // Compile-time check that all stage types are accessible
        use crate::pipeline::{Researcher, ScenarioGen, Reporter};
        let _: &str = Researcher::STAGE_NAME;
        let _: &str = ScenarioGen::STAGE_NAME;
        let _: &str = Reporter::STAGE_NAME;
    }
}
```

Run — expect compile failure:
```bash
cargo test -p agent007-simulation 2>&1 | head -20
```

**Step 2 — implement**

`crates/simulation/src/pipeline.rs`:
```rust
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use agent007_core::dispatcher::Dispatcher;
use agent007_memory::MemoryStore;
use agent007_models::provider::ModelProvider;

use crate::error::SimulationError;
use crate::simulator::Simulator;
use crate::types::{ScenarioFailure, SimulationReport, SimulationTemplate};

// Memory key for persisting the last report
const MEMORY_KEY_PREFIX: &str = "simulation/report/";

// ---------------------------------------------------------------------------
// Stage 1: Researcher
// ---------------------------------------------------------------------------

/// Uses the AI model to gather context on the simulation domain.
pub struct Researcher {
    pub provider: Arc<dyn ModelProvider>,
    pub model: String,
}

impl Researcher {
    pub const STAGE_NAME: &'static str = "Researcher";

    pub async fn run(&self, template: &SimulationTemplate) -> Result<String, SimulationError> {
        if template.research_topics.is_empty() {
            return Ok(String::new());
        }

        let topics = template.research_topics.join(", ");
        let prompt = format!(
            "You are a domain researcher. Summarise the key concepts for a simulation of '{}'. \
             Cover the following topics concisely (≤ 300 words total): {topics}.",
            template.name
        );

        let req = agent007_models::types::CompletionRequest {
            model: self.model.clone(),
            messages: vec![agent007_models::types::Message {
                role: agent007_models::types::Role::User,
                content: prompt,
            }],
            stream: false,
        };

        self.provider
            .complete(req)
            .await
            .map(|r| r.content)
            .map_err(|e| SimulationError::ModelError(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Stage 2: ScenarioGen
// ---------------------------------------------------------------------------

/// Optionally generates additional scenario definitions using the AI model.
/// If the template already has scenarios defined, returns them unchanged.
pub struct ScenarioGen {
    pub provider: Arc<dyn ModelProvider>,
    pub model: String,
}

impl ScenarioGen {
    pub const STAGE_NAME: &'static str = "ScenarioGen";

    pub async fn run(
        &self,
        template: &SimulationTemplate,
        research_context: &str,
    ) -> Result<Vec<crate::types::ScenarioDef>, SimulationError> {
        // If the template already declares scenarios, respect them exactly
        if !template.scenarios.is_empty() {
            return Ok(template.scenarios.clone());
        }

        let prompt = format!(
            "You are a simulation scenario generator. Based on the following research context \
             and simulation template, produce a JSON array of scenario objects. Each object must \
             have a 'name' field (string) and domain-specific parameter fields.\n\
             Template: {}\nResearch context:\n{research_context}",
            template.name
        );

        let req = agent007_models::types::CompletionRequest {
            model: self.model.clone(),
            messages: vec![agent007_models::types::Message {
                role: agent007_models::types::Role::User,
                content: prompt,
            }],
            stream: false,
        };

        let resp = self
            .provider
            .complete(req)
            .await
            .map_err(|e| SimulationError::ModelError(e.to_string()))?;

        let json = extract_json_block(&resp.content);
        serde_json::from_str(&json).map_err(|e| SimulationError::ParseError {
            path: std::path::PathBuf::from("<ai-generated>"),
            reason: e.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Stage 3: Simulator (invokes the SUT)
// ---------------------------------------------------------------------------
// The Simulator struct lives in simulator.rs and is used directly by the pipeline.

// ---------------------------------------------------------------------------
// Stage 4: Validator
// ---------------------------------------------------------------------------

/// Validates each scenario output against the template's ValidationConfig.
pub struct Validator;

impl Validator {
    pub const STAGE_NAME: &'static str = "Validator";

    pub fn run(
        &self,
        template: &SimulationTemplate,
        scenario_outputs: &[(String, String)],  // (scenario_name, raw_output)
    ) -> Vec<ScenarioFailure> {
        let sim = Simulator::new();
        let mut failures = Vec::new();

        for (name, output) in scenario_outputs {
            if let Err(e) = sim.validate_output(name, output, &template.validation) {
                failures.push(ScenarioFailure {
                    scenario: name.clone(),
                    reason: e.to_string(),
                });
            }
        }

        failures
    }
}

// ---------------------------------------------------------------------------
// Stage 5: Reporter
// ---------------------------------------------------------------------------

/// Builds the SimulationReport, detects regressions, and persists to memory.
pub struct Reporter {
    pub memory: Arc<MemoryStore>,
}

impl Reporter {
    pub const STAGE_NAME: &'static str = "Reporter";

    pub fn run(
        &self,
        template_name: &str,
        scenarios_run: usize,
        failures: Vec<ScenarioFailure>,
    ) -> Result<SimulationReport, SimulationError> {
        let previous = self.load_previous(template_name);
        let regressions = Self::detect_regressions(&failures, previous.as_ref());
        let scenarios_failed = failures.len();
        let scenarios_passed = scenarios_run.saturating_sub(scenarios_failed);

        let report = SimulationReport {
            run_id: Uuid::new_v4().to_string(),
            template_name: template_name.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            scenarios_run,
            scenarios_passed,
            scenarios_failed,
            failures,
            regressions,
        };

        self.store(&report)?;
        Ok(report)
    }

    fn memory_key(template_name: &str) -> String {
        format!("{MEMORY_KEY_PREFIX}{template_name}")
    }

    fn load_previous(&self, template_name: &str) -> Option<SimulationReport> {
        let raw = self
            .memory
            .read(&Self::memory_key(template_name))
            .ok()??;
        serde_json::from_str(&raw).ok()
    }

    fn store(&self, report: &SimulationReport) -> Result<(), SimulationError> {
        let json = serde_json::to_string_pretty(report)
            .map_err(|e| SimulationError::ParseError {
                path: std::path::PathBuf::from("<memory>"),
                reason: e.to_string(),
            })?;
        self.memory
            .write(&Self::memory_key(&report.template_name), &json)
            .map_err(|e| SimulationError::Io(std::io::Error::other(e.to_string())))
    }

    fn detect_regressions(
        current_failures: &[ScenarioFailure],
        previous: Option<&SimulationReport>,
    ) -> Vec<String> {
        let prev = match previous {
            Some(p) => p,
            None => return vec![],
        };
        let previously_failed: std::collections::HashSet<&str> =
            prev.failures.iter().map(|f| f.scenario.as_str()).collect();

        current_failures
            .iter()
            .filter(|f| !previously_failed.contains(f.scenario.as_str()))
            .map(|f| f.scenario.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// SimulationPipeline — orchestrates all stages
// ---------------------------------------------------------------------------

pub struct SimulationPipeline {
    pub provider: Arc<dyn ModelProvider>,
    pub memory: Arc<MemoryStore>,
    pub dispatcher: Arc<dyn Dispatcher>,
}

impl SimulationPipeline {
    pub async fn run(
        &self,
        template: &SimulationTemplate,
    ) -> Result<SimulationReport, SimulationError> {
        tracing::info!(template = %template.name, "SimulationPipeline: starting");

        // Stage 1: Research
        let researcher = Researcher {
            provider: Arc::clone(&self.provider),
            model: "claude".into(),
        };
        let research = researcher.run(template).await?;
        tracing::info!("SimulationPipeline: research complete");

        // Stage 2: ScenarioGen
        let scenario_gen = ScenarioGen {
            provider: Arc::clone(&self.provider),
            model: "claude".into(),
        };
        let scenarios = scenario_gen.run(template, &research).await?;
        tracing::info!(count = scenarios.len(), "SimulationPipeline: scenarios ready");

        // Stage 3: Simulate — run each scenario
        let sim = Simulator::new();
        let mut scenario_outputs: Vec<(String, String)> = Vec::new();
        let mut sut_failures: Vec<ScenarioFailure> = Vec::new();

        for scenario in &scenarios {
            match sim.run_scenario(&template.system_under_test, scenario).await {
                Ok(output) => {
                    scenario_outputs.push((scenario.name.clone(), output));
                }
                Err(SimulationError::Timeout { name, secs }) => {
                    tracing::warn!(scenario = %name, secs, "scenario timed out");
                    sut_failures.push(ScenarioFailure {
                        scenario: name,
                        reason: format!("timed out after {secs}s"),
                    });
                }
                Err(SimulationError::SutFailed { code, stderr }) => {
                    tracing::warn!(scenario = %scenario.name, code, "SUT failed");
                    sut_failures.push(ScenarioFailure {
                        scenario: scenario.name.clone(),
                        reason: format!("exit code {code}: {stderr}"),
                    });
                }
                Err(e) => return Err(e),
            }
        }
        tracing::info!(
            run = scenario_outputs.len(),
            failed = sut_failures.len(),
            "SimulationPipeline: simulation complete"
        );

        // Stage 4: Validate
        let validator = Validator;
        let mut validation_failures = validator.run(template, &scenario_outputs);
        validation_failures.extend(sut_failures);

        tracing::info!(
            failures = validation_failures.len(),
            "SimulationPipeline: validation complete"
        );

        // Stage 5: Report
        let reporter = Reporter {
            memory: Arc::clone(&self.memory),
        };
        let report = reporter.run(&template.name, scenarios.len(), validation_failures)?;

        tracing::info!(
            passed = report.scenarios_passed,
            failed = report.scenarios_failed,
            regressions = report.regressions.len(),
            "SimulationPipeline: complete"
        );

        Ok(report)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_json_block(text: &str) -> String {
    if let Some(start) = text.find("```json") {
        if let Some(end) = text[start + 7..].find("```") {
            return text[start + 7..start + 7 + end].trim().to_string();
        }
    }
    if let Some(start) = text.find("```") {
        if let Some(end) = text[start + 3..].find("```") {
            return text[start + 3..start + 3 + end].trim().to_string();
        }
    }
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_report(failures: &[&str]) -> SimulationReport {
        SimulationReport {
            run_id: "r".into(),
            template_name: "t".into(),
            timestamp: "ts".into(),
            scenarios_run: 3,
            scenarios_passed: 3 - failures.len(),
            scenarios_failed: failures.len(),
            failures: failures
                .iter()
                .map(|s| ScenarioFailure {
                    scenario: s.to_string(),
                    reason: "err".into(),
                })
                .collect(),
            regressions: vec![],
        }
    }

    #[test]
    fn pipeline_stage_names_are_ordered() {
        assert_eq!(Researcher::STAGE_NAME, "Researcher");
        assert_eq!(ScenarioGen::STAGE_NAME, "ScenarioGen");
        assert_eq!(Validator::STAGE_NAME, "Validator");
        assert_eq!(Reporter::STAGE_NAME, "Reporter");
    }

    #[test]
    fn reporter_detect_regressions_finds_new_failures() {
        let previous = make_report(&[]);
        let current_failures = vec![ScenarioFailure {
            scenario: "open_office".into(),
            reason: "error_m too high".into(),
        }];
        let regressions = Reporter::detect_regressions(&current_failures, Some(&previous));
        assert_eq!(regressions, vec!["open_office"]);
    }

    #[test]
    fn reporter_no_regression_for_pre_existing_failure() {
        let previous = make_report(&["open_office"]);
        let current_failures = vec![ScenarioFailure {
            scenario: "open_office".into(),
            reason: "still failing".into(),
        }];
        let regressions = Reporter::detect_regressions(&current_failures, Some(&previous));
        assert!(regressions.is_empty());
    }

    #[test]
    fn reporter_store_and_load() {
        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(MemoryStore::new(tmp.path()));
        let reporter = Reporter { memory: Arc::clone(&memory) };

        let report = reporter.run("wifi-rtt", 3, vec![]).unwrap();
        assert_eq!(report.scenarios_run, 3);
        assert_eq!(report.template_name, "wifi-rtt");

        // Load back via memory directly
        let raw = memory
            .read(&Reporter::memory_key("wifi-rtt"))
            .unwrap()
            .unwrap();
        let loaded: SimulationReport = serde_json::from_str(&raw).unwrap();
        assert_eq!(loaded.run_id, report.run_id);
    }

    #[test]
    fn validator_passes_empty_output() {
        use crate::types::ValidationConfig;
        let template = SimulationTemplate {
            name: "t".into(),
            description: None,
            research_topics: vec![],
            system_under_test: crate::types::SystemUnderTest {
                command: "echo".into(),
                args: vec![],
                timeout_secs: None,
                working_dir: None,
                env: None,
            },
            scenarios: vec![],
            validation: ValidationConfig::default(),
            output: Default::default(),
        };
        let validator = Validator;
        let failures = validator.run(&template, &[("s1".into(), "{}".into())]);
        assert!(failures.is_empty());
    }
}
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-simulation
```

- [ ] Task 6 complete

---

## Chunk 7: CLI — replace stub with full implementation

### Task 7: Replace the existing simulate.rs stub with the real implementation

**Files:**
- Modify: `crates/cli/src/commands/simulate.rs` — replace stub with real implementation
- Modify: `crates/cli/Cargo.toml` — add `agent007-simulation = { path = "../simulation" }`
- Modify: `crates/cli/src/main.rs` — extend `Simulate` variant with richer CLI structure

**Step 1 — write failing tests**

Add to the bottom of the new `simulate.rs` before writing it:
```rust
#[cfg(test)]
mod tests {
    use clap::Parser;
    use super::SimulateArgs;

    #[test]
    fn simulate_list_parses() {
        // Will fail until SimulateArgs is defined
        let args = SimulateArgs::try_parse_from(["simulate", "list"]).unwrap();
        assert!(matches!(args.action, super::SimulateAction::List));
    }
}
```

Run — expect compile failure:
```bash
cargo test -p agent007 2>&1 | head -20
```

**Step 2 — implement**

`crates/cli/src/commands/simulate.rs` (replace the entire existing stub):
```rust
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};

use agent007_core::dispatcher::LocalDispatcher;
use agent007_memory::MemoryStore;
use agent007_models::MockProvider;
use agent007_simulation::{SimulationPipeline, TemplateLoader};

use crate::config::Config;

#[derive(Parser, Debug)]
pub struct SimulateArgs {
    #[command(subcommand)]
    pub action: SimulateAction,
}

#[derive(Subcommand, Debug)]
pub enum SimulateAction {
    /// Run a named simulation template
    #[command(external_subcommand)]
    Run(Vec<String>),
    /// List all available simulation templates
    List,
    /// Show the stored simulation report
    Report {
        /// Show the last stored report (for the given template, or most recent)
        #[arg(long)]
        last: bool,
        /// Show only regression scenario names
        #[arg(long)]
        regressions: bool,
        /// Template name to show report for
        #[arg(long)]
        template: Option<String>,
    },
}

pub async fn execute(_config: Arc<Config>, args: SimulateArgs) -> Result<()> {
    match args.action {
        SimulateAction::Run(parts) => {
            run_simulation(parts).await?;
        }

        SimulateAction::List => {
            let loader = TemplateLoader::new_builtin_only();
            let names = loader.list();
            if names.is_empty() {
                println!("No simulation templates found.");
            } else {
                println!("Available templates:");
                for name in &names {
                    println!("  {name}");
                }
            }
        }

        SimulateAction::Report { regressions, template, .. } => {
            show_report(template, regressions)?;
        }
    }

    Ok(())
}

/// Run a simulation given arguments like `["wifi-rtt"]` or `["wifi-roaming", "--scenario", "fast_vehicle_highway"]`.
async fn run_simulation(parts: Vec<String>) -> Result<()> {
    let template_name = parts
        .first()
        .ok_or_else(|| anyhow::anyhow!("Usage: agent007 simulate <template-name>"))?
        .clone();

    // Optional --scenario filter
    let scenario_filter: Option<String> = {
        let mut it = parts.iter().skip(1);
        let mut found = None;
        while let Some(arg) = it.next() {
            if arg == "--scenario" {
                found = it.next().cloned();
                break;
            }
        }
        found
    };

    let loader = TemplateLoader::new_builtin_only();
    let mut template = loader.load(&template_name)?;

    if let Some(filter) = scenario_filter {
        template.scenarios.retain(|s| s.name == filter);
        if template.scenarios.is_empty() {
            anyhow::bail!("No scenario named '{}' in template '{}'", filter, template_name);
        }
    }

    let memory_dir = home_dir().join(".agent007").join("memory");
    let memory = Arc::new(MemoryStore::new(&memory_dir));
    let dispatcher = LocalDispatcher::new(64);

    // Use MockProvider as placeholder — replace with configured live provider
    // once provider resolution from agent007-core is wired up (see plan 1).
    let provider = Arc::new(MockProvider::default());

    let pipeline = SimulationPipeline {
        provider,
        memory,
        dispatcher,
    };

    let report = pipeline.run(&template).await?;

    println!("Run ID   : {}", report.run_id);
    println!("Template : {}", report.template_name);
    println!("Scenarios: {} run, {} passed, {} failed",
             report.scenarios_run, report.scenarios_passed, report.scenarios_failed);

    if !report.regressions.is_empty() {
        println!("\nRegressions ({}):", report.regressions.len());
        for r in &report.regressions {
            println!("  - {r}");
        }
    }

    if !report.failures.is_empty() {
        println!("\nFailures:");
        for f in &report.failures {
            println!("  [FAIL] {} — {}", f.scenario, f.reason);
        }
    } else {
        println!("\nAll scenarios passed.");
    }

    Ok(())
}

fn show_report(template_name: Option<String>, regressions_only: bool) -> Result<()> {
    let memory_dir = home_dir().join(".agent007").join("memory");
    let memory = Arc::new(MemoryStore::new(&memory_dir));

    let key = match &template_name {
        Some(n) => format!("simulation/report/{n}"),
        None => {
            anyhow::bail!(
                "Specify --template <name>, e.g. `agent007 simulate report --last --template wifi-rtt`"
            );
        }
    };

    match memory.read(&key)? {
        None => println!("No report found for '{}'. Run `agent007 simulate {}` first.",
                         template_name.unwrap_or_default(), ""),
        Some(raw) => {
            let report: agent007_simulation::SimulationReport = serde_json::from_str(&raw)?;
            if regressions_only {
                if report.regressions.is_empty() {
                    println!("No regressions.");
                } else {
                    for r in &report.regressions {
                        println!("{r}");
                    }
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
        }
    }

    Ok(())
}

fn home_dir() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn simulate_list_parses() {
        let args = SimulateArgs::try_parse_from(["simulate", "list"]).unwrap();
        assert!(matches!(args.action, SimulateAction::List));
    }

    #[test]
    fn simulate_report_with_regressions_flag() {
        let args = SimulateArgs::try_parse_from([
            "simulate", "report", "--last", "--regressions", "--template", "wifi-rtt",
        ])
        .unwrap();
        assert!(matches!(
            args.action,
            SimulateAction::Report { regressions: true, .. }
        ));
    }

    #[test]
    fn simulate_run_with_template_parses() {
        let args = SimulateArgs::try_parse_from(["simulate", "wifi-rtt"]).unwrap();
        assert!(matches!(args.action, SimulateAction::Run(_)));
    }
}
```

**Modify `crates/cli/src/main.rs`** — update the `Simulate` variant to accept the richer `SimulateArgs`:
```rust
// Replace:
Simulate {
    template: String,
},
// With:
Simulate(SimulateArgs),

// Replace dispatch:
Commands::Simulate { template } => commands::simulate::execute(config, template).await?,
// With:
Commands::Simulate(args) => commands::simulate::execute(config, args).await?,
```

Also add `use crate::commands::simulate::SimulateArgs;` near other command arg imports.

**Modify `crates/cli/Cargo.toml`** — add dependency:
```toml
agent007-simulation = { path = "../simulation" }
```

**Step 3 — run tests (green)**
```bash
cargo test -p agent007-simulation
cargo test -p agent007
```

- [ ] Task 7 complete

---

## Chunk 8: Integration smoke test

### Task 8: End-to-end smoke test — run wifi-rtt template against a mock SUT

**Files:**
- Modify: `crates/simulation/src/pipeline.rs` — add ignored integration test

**Step 1 — write failing test**

The test uses `/usr/bin/true` as the SUT (always exits 0, no meaningful output) to verify the full pipeline wires up without real domain logic:

```rust
#[tokio::test]
#[ignore = "integration test — requires /usr/bin/true available"]
async fn pipeline_runs_wifi_rtt_with_mock_sut() {
    use std::sync::Arc;
    use tempfile::TempDir;
    use agent007_core::dispatcher::LocalDispatcher;
    use agent007_memory::MemoryStore;
    use agent007_models::MockProvider;
    use crate::loader::TemplateLoader;

    let tmp = TempDir::new().unwrap();
    let memory = Arc::new(MemoryStore::new(tmp.path()));
    let dispatcher = LocalDispatcher::new(64);
    let provider = Arc::new(MockProvider::default());

    let mut loader = TemplateLoader::new_builtin_only();
    let mut template = loader.load("wifi-rtt").unwrap();

    // Override SUT with a no-op command to avoid needing a real positioning binary
    template.system_under_test.command = "true".into();
    template.system_under_test.args = vec![];

    let pipeline = SimulationPipeline { provider, memory, dispatcher };
    let report = pipeline.run(&template).await.unwrap();
    assert_eq!(report.template_name, "wifi-rtt");
    assert!(report.scenarios_run > 0);
}
```

Run:
```bash
cargo test -p agent007-simulation -- --ignored pipeline_runs_wifi_rtt_with_mock_sut
```

**Step 2** — No new implementation needed; wired in Task 6.

**Step 3 — verify all tests still green**
```bash
cargo test -p agent007-simulation
cargo test -p agent007
```

- [ ] Task 8 complete

---

## Full test command reference

```bash
# Run all tests in the simulation crate
cargo test -p agent007-simulation

# Run all tests including CLI
cargo test -p agent007

# Run the full workspace
cargo test --workspace

# Run the integration test
cargo test -p agent007-simulation -- --ignored pipeline_runs_wifi_rtt_with_mock_sut

# Manual CLI smoke tests (after cargo build)
agent007 simulate list
agent007 simulate wifi-rtt
agent007 simulate wifi-roaming --scenario slow_walk_office
agent007 simulate report --last --template wifi-rtt
agent007 simulate report --regressions --template wifi-roaming
```

---

## Summary checklist

- [ ] Chunk 1 — Scaffold crate + error type
- [ ] Chunk 2 — Core types (SimulationTemplate, SimulationReport, etc.)
- [ ] Chunk 3 — Built-in templates (wifi-rtt.toml + wifi-roaming.toml) + embed via include_str!
- [ ] Chunk 4 — TemplateLoader (built-in + custom resolution)
- [ ] Chunk 5 — Simulator (SUT invocation, temp-file I/O, timeout, validation)
- [ ] Chunk 6 — SimulationPipeline (all 5 stages: Researcher, ScenarioGen, Simulator, Validator, Reporter)
- [ ] Chunk 7 — CLI: replace simulate.rs stub with full SimulateArgs + SimulateAction
- [ ] Chunk 8 — Integration smoke test
