use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use agent007_core::RunStore;
use agent007_mcp::{McpClient, McpServerConfig};
use serde_json::{json, Value};

fn write_fixture_project(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "fixture-project"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    fs::write(
        root.join("README.md"),
        "# Fixture Project\n\nThis repo exercises compact context and budget tooling.\n",
    )
    .unwrap();
    fs::write(
        root.join("AGENTS.md"),
        "Prefer cargo test\nKeep diffs concise\nPersist high-signal artifacts\n",
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        r#"pub fn compact_fixture() -> &'static str {
    "fixture"
}

#[cfg(test)]
mod tests {
    #[test]
    fn works() {
        assert_eq!(super::compact_fixture(), "fixture");
    }
}
"#,
    )
    .unwrap();
}

fn write_agent_home(home: &Path) {
    fs::create_dir_all(home.join("workflows")).unwrap();
    fs::create_dir_all(home.join("skills")).unwrap();
    fs::create_dir_all(home.join("memory").join("project")).unwrap();
    fs::write(home.join("workflows").join("ship.toml"), "name = 'ship'\n").unwrap();
    fs::write(
        home.join("skills").join("review.md"),
        "---\ntrigger: /review\nprompt: Review {{ args }}\n---\n",
    )
    .unwrap();
    fs::write(
        home.join("memory").join("project").join("auth.md"),
        "# Auth\nUse context compilation before broad repository scans.\n",
    )
    .unwrap();
}

fn write_test_config(path: &Path) {
    fs::write(
        path,
        r#"[models]
default = "mock"
"#,
    )
    .unwrap();
}

fn extract_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => items.iter().find_map(extract_text),
        Value::Object(map) => map
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                map.values().find_map(|entry| match entry {
                    Value::String(text) => Some(text.clone()),
                    Value::Object(inner) => inner
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    _ => None,
                })
            }),
        _ => None,
    }
}

fn extract_json(value: Value) -> Value {
    let text = extract_text(&value).expect("MCP tool result should contain text");
    serde_json::from_str(&text).expect("MCP tool text should be valid JSON")
}

#[tokio::test]
async fn live_mcp_server_exposes_and_records_new_compact_context_tools() {
    let workspace = tempfile::tempdir().unwrap();
    let project_root = workspace.path().join("fixture-project");
    let agent_home = workspace.path().join("agent-home");
    let config_path = workspace.path().join("config.toml");

    fs::create_dir_all(&project_root).unwrap();
    fs::create_dir_all(&agent_home).unwrap();
    write_fixture_project(&project_root);
    write_agent_home(&agent_home);
    write_test_config(&config_path);

    let mut env = HashMap::new();
    env.insert(
        "AGENT007_HOME".to_string(),
        agent_home.display().to_string(),
    );
    env.insert(
        "AGENT007_CONFIG".to_string(),
        config_path.display().to_string(),
    );
    env.insert("RUST_LOG".to_string(), "warn".to_string());

    let mut client = McpClient::new(vec![McpServerConfig {
        name: "agent007".to_string(),
        command: env!("CARGO_BIN_EXE_agent007").to_string(),
        args: vec!["serve".to_string(), "--no-dashboard".to_string()],
        env,
        cwd: Some(project_root.display().to_string()),
    }]);

    client.connect().await.unwrap();

    let tools = client.list_tools().await.unwrap();
    let tool_names: Vec<String> = tools.into_iter().map(|tool| tool.name).collect();
    for expected in [
        "agent007_compact_output",
        "agent007_context_compile",
        "agent007_repo_brain_refresh",
        "agent007_budget_estimate",
    ] {
        assert!(
            tool_names.iter().any(|name| name == expected),
            "missing MCP tool {expected:?}"
        );
    }

    let store = RunStore::new(agent_home.join("sessions"));

    let compact = extract_json(
        client
            .call_tool(
                "agent007_compact_output",
                json!({
                    "command": "cargo test",
                    "output": "warning: one\ntest auth::works ... FAILED\ntest result: FAILED. 1 passed; 1 failed",
                    "level": "compact"
                }),
            )
            .await
            .unwrap(),
    );
    let compact_run_id = compact["run_id"].as_str().unwrap();
    assert_eq!(compact["result"]["strategy"], "test-output");
    assert_eq!(compact["result"]["level"], "compact");
    let compact_run = store.load_run(compact_run_id).unwrap();
    assert!(compact_run
        .artifacts
        .contains(&"compact-output.json".to_string()));
    assert!(compact_run
        .artifacts
        .contains(&"raw-output.txt".to_string()));
    let compact_text = store
        .read_text_artifact(compact_run_id, "compact-output.txt")
        .unwrap();
    assert!(compact_text.contains("Test output summary"));
    assert!(compact_text.contains("Failures: 1"));

    let context = extract_json(
        client
            .call_tool(
                "agent007_context_compile",
                json!({
                    "task": "tighten compact output tests for the fixture project",
                    "max_files": 3,
                    "max_memory_notes": 2,
                    "max_prompt_tokens": 1200,
                    "reserve_tokens": 200,
                    "max_response_tokens": 300
                }),
            )
            .await
            .unwrap(),
    );
    let context_run_id = context["run_id"].as_str().unwrap();
    assert_eq!(
        context["bundle"]["repo_brain"]["project_name"],
        "fixture-project"
    );
    assert!(context["bundle"]["compiled_context"]
        .as_str()
        .unwrap()
        .contains("Task:"));
    assert!(
        context["bundle"]["relevant_files"]
            .as_array()
            .unwrap()
            .len()
            <= 3
    );
    let context_text = store
        .read_text_artifact(context_run_id, "compiled-context.txt")
        .unwrap();
    assert!(context_text.contains("Repo brain:"));

    let repo_brain = extract_json(
        client
            .call_tool("agent007_repo_brain_refresh", json!({}))
            .await
            .unwrap(),
    );
    let repo_brain_run_id = repo_brain["run_id"].as_str().unwrap();
    assert_eq!(repo_brain["memory_key"], "project/repo_brain");
    assert_eq!(
        repo_brain["repo_brain"]["project_name"].as_str().unwrap(),
        "fixture-project"
    );
    // Read back via the store API (SQLite-backed; no .md file on disk).
    let mem_store = Arc::new(agent007_memory::MemoryStore::new(agent_home.join("memory")));
    let repo_brain_note = mem_store
        .scoped("project")
        .read("repo_brain")
        .unwrap()
        .unwrap();
    assert!(repo_brain_note.contains("# Repo Brain: fixture-project"));
    let repo_brain_markdown = store
        .read_text_artifact(repo_brain_run_id, "repo-brain.md")
        .unwrap();
    assert!(repo_brain_markdown.contains("## Recommended Commands"));

    let budget = extract_json(
        client
            .call_tool(
                "agent007_budget_estimate",
                json!({
                    "task": "estimate a large prompt",
                    "text": "x".repeat(30_000),
                    "max_prompt_tokens": 2000,
                    "reserve_tokens": 500,
                    "max_response_tokens": 400
                }),
            )
            .await
            .unwrap(),
    );
    let budget_run_id = budget["run_id"].as_str().unwrap();
    assert_eq!(budget["report"]["recommended_level"], "aggressive");
    assert!(budget["report"]["should_use_artifacts"].as_bool().unwrap());
    let budget_report = store
        .read_json_artifact::<Value>(budget_run_id, "budget-report.json")
        .unwrap();
    assert_eq!(budget_report["recommended_level"], "aggressive");

    let recent_runs = store.list_runs(10).unwrap();
    assert!(recent_runs.len() >= 4);

    drop(client);
}
