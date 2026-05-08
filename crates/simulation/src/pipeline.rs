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
             Cover the following topics concisely (\u{2264} 300 words total): {topics}.",
            template.name
        );

        let req = agent007_models::types::CompletionRequest {
            model: self.model.clone(),
            messages: vec![agent007_models::types::Message {
                role: agent007_models::types::Role::User,
                content: prompt,
            }],
            max_tokens: None,
            temperature: None,
            system: None,
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
            max_tokens: None,
            temperature: None,
            system: None,
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
        scenario_outputs: &[(String, String)], // (scenario_name, raw_output)
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

    pub fn memory_key(template_name: &str) -> String {
        format!("{MEMORY_KEY_PREFIX}{template_name}")
    }

    fn load_previous(&self, template_name: &str) -> Option<SimulationReport> {
        let raw = self.memory.read(&Self::memory_key(template_name)).ok()??;
        serde_json::from_str(&raw).ok()
    }

    fn store(&self, report: &SimulationReport) -> Result<(), SimulationError> {
        let json =
            serde_json::to_string_pretty(report).map_err(|e| SimulationError::ParseError {
                path: std::path::PathBuf::from("<memory>"),
                reason: e.to_string(),
            })?;
        self.memory
            .write(&Self::memory_key(&report.template_name), &json)
            .map_err(|e| SimulationError::Io(std::io::Error::other(e.to_string())))
    }

    pub fn detect_regressions(
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
        tracing::info!(
            count = scenarios.len(),
            "SimulationPipeline: scenarios ready"
        );

        // Stage 3: Simulate — run each scenario
        let sim = Simulator::new();
        let mut scenario_outputs: Vec<(String, String)> = Vec::new();
        let mut sut_failures: Vec<ScenarioFailure> = Vec::new();

        for scenario in &scenarios {
            match sim
                .run_scenario(&template.system_under_test, scenario)
                .await
            {
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
        let reporter = Reporter {
            memory: Arc::clone(&memory),
        };

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

    #[tokio::test]
    #[ignore = "integration test — requires cargo build to be available"]
    async fn pipeline_runs_skills_smoke_with_mock_sut() {
        use crate::loader::TemplateLoader;

        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(MemoryStore::new(tmp.path()));
        let dispatcher = agent007_core::dispatcher::LocalDispatcher::new(64);
        let provider = Arc::new(agent007_models::MockProvider::new("", "mock"));

        let loader = TemplateLoader::new_builtin_only();
        let mut template = loader.load("skills-smoke").unwrap();

        // Override SUT with a no-op command to avoid needing a real cargo build
        template.system_under_test.command = "echo".into();
        template.system_under_test.args = vec![
            "/brainstorm /dev-architect /dev-debug /code-refactor".into(),
        ];

        let pipeline = SimulationPipeline {
            provider,
            memory,
            dispatcher,
        };
        let report = pipeline.run(&template).await.unwrap();
        assert_eq!(report.template_name, "skills-smoke");
        assert!(report.scenarios_run > 0);
    }
}
