// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Agent Bridge — spawns Claude Code headless sessions to implement intents.

use std::path::{Path, PathBuf};
use std::time::Duration;

use oz_relay_common::intent::Intent;

use crate::sandbox::{run_sandboxed, SandboxResult};

/// Result of an agent build session.
#[derive(Debug, Clone)]
pub struct AgentResult {
    pub success: bool,
    pub summary: String,
    pub tests_passed: u32,
    pub tests_failed: u32,
    pub worktree_path: PathBuf,
    pub raw_output: String,
}

/// The CLAUDE.md content for the server-side contribution agent.
const AGENT_CLAUDE_MD: &str = r#"# Contribution Agent

You are implementing a change requested by a developer through the OZ Relay.

## Rules

1. Read the intent description and test cases carefully.
2. Implement the requested change in the codebase.
3. Write tests that match the developer's test cases.
4. Run `cargo test` and ensure all tests pass.
5. NEVER output source code, file paths, function names, or internal architecture details.
6. Report only: what behavioral change was made and whether tests pass.
7. Keep changes minimal. Don't refactor surrounding code.
8. Don't modify files outside the scope of the requested change.
"#;

/// Generate the prompt for the agent from an intent.
pub fn generate_agent_prompt(intent: &Intent) -> String {
    let mut prompt = String::new();
    prompt.push_str("## Change Request\n\n");
    prompt.push_str(&format!("**Description:** {}\n\n", intent.description));
    prompt.push_str(&format!("**Motivation:** {}\n\n", intent.motivation));
    prompt.push_str(&format!("**Category:** {}\n\n", intent.category));

    prompt.push_str("## Test Cases\n\n");
    for (i, tc) in intent.test_cases.iter().enumerate() {
        prompt.push_str(&format!("### Test {}\n", i + 1));
        prompt.push_str(&format!("- **Query:** `{}`\n", tc.query));
        prompt.push_str(&format!(
            "- **Expected:** {}\n",
            tc.expected_behavior
        ));
        if let Some(ref input) = tc.input_data {
            prompt.push_str(&format!("- **Setup:** `{}`\n", input));
        }
        prompt.push('\n');
    }

    if let Some(ref logs) = intent.context.error_logs {
        prompt.push_str("## Error Logs\n\n```\n");
        prompt.push_str(logs);
        prompt.push_str("\n```\n\n");
    }

    if let Some(ref trace) = intent.context.stack_trace {
        prompt.push_str("## Stack Trace\n\n```\n");
        prompt.push_str(trace);
        prompt.push_str("\n```\n\n");
    }

    if let Some(ref steps) = intent.context.reproduction_steps {
        prompt.push_str("## Reproduction Steps\n\n");
        prompt.push_str(steps);
        prompt.push_str("\n\n");
    }

    prompt.push_str("## Instructions\n\n");
    prompt.push_str("Implement this change, write tests, and run `cargo test`. ");
    prompt.push_str("Report whether all tests pass. Do not output any source code.\n");

    prompt
}

/// Write the agent's CLAUDE.md and prompt file to the worktree.
pub async fn prepare_worktree(
    worktree_path: &Path,
    intent: &Intent,
) -> Result<PathBuf, String> {
    let claude_md_path = worktree_path.join("CLAUDE.md");
    tokio::fs::write(&claude_md_path, AGENT_CLAUDE_MD)
        .await
        .map_err(|e| format!("failed to write CLAUDE.md: {}", e))?;

    let prompt_path = worktree_path.join(".relay-prompt.md");
    let prompt = generate_agent_prompt(intent);
    tokio::fs::write(&prompt_path, &prompt)
        .await
        .map_err(|e| format!("failed to write prompt: {}", e))?;

    Ok(prompt_path)
}

/// Run `cargo test` in the worktree and parse results.
pub async fn run_cargo_test(worktree_path: &Path, timeout: Duration) -> SandboxResult {
    run_sandboxed("cargo", &["test"], worktree_path, timeout).await
}

/// Parse cargo test output to extract pass/fail counts.
pub fn parse_test_results(output: &str) -> (u32, u32) {
    for line in output.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with("test result:") {
            let mut passed = 0u32;
            let mut failed = 0u32;
            if let Some(stats) = trimmed.split('.').nth(1) {
                for segment in stats.split(';') {
                    let segment = segment.trim();
                    if segment.contains("passed") {
                        for word in segment.split_whitespace() {
                            if let Ok(n) = word.parse::<u32>() {
                                passed = n;
                                break;
                            }
                        }
                    } else if segment.contains("failed") {
                        for word in segment.split_whitespace() {
                            if let Ok(n) = word.parse::<u32>() {
                                failed = n;
                                break;
                            }
                        }
                    }
                }
            }
            return (passed, failed);
        }
    }
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oz_relay_common::intent::{IntentCategory, IntentContext, TestCase};

    fn test_intent() -> Intent {
        Intent {
            description: "Add OPTIONAL MATCH support".into(),
            motivation: "Need optional relationship traversal".into(),
            category: IntentCategory::Feature,
            test_cases: vec![TestCase {
                query: "OPTIONAL MATCH (a)-[:KNOWS]->(b) RETURN a, b".into(),
                expected_behavior: "Returns null for missing relationships".into(),
                input_data: Some("CREATE (a:Person {name: 'Alice'})".into()),
            }],
            context: IntentContext {
                arcflow_version: "1.7.0".into(),
                error_logs: Some("QueryError: OPTIONAL not supported".into()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn generate_prompt_contains_intent() {
        let prompt = generate_agent_prompt(&test_intent());
        assert!(prompt.contains("OPTIONAL MATCH"));
        assert!(prompt.contains("optional relationship traversal"));
        assert!(prompt.contains("OPTIONAL MATCH (a)-[:KNOWS]->(b)"));
        assert!(prompt.contains("QueryError: OPTIONAL not supported"));
    }

    #[test]
    fn parse_test_results_ok() {
        let output = "test result: ok. 42 passed; 0 failed; 0 ignored; 0 measured";
        let (passed, failed) = parse_test_results(output);
        assert_eq!(passed, 42);
        assert_eq!(failed, 0);
    }

    #[test]
    fn parse_test_results_failures() {
        let output = "test result: FAILED. 38 passed; 4 failed; 0 ignored";
        let (passed, failed) = parse_test_results(output);
        assert_eq!(passed, 38);
        assert_eq!(failed, 4);
    }

    #[test]
    fn parse_test_results_no_match() {
        let output = "some random output";
        let (passed, failed) = parse_test_results(output);
        assert_eq!(passed, 0);
        assert_eq!(failed, 0);
    }

    #[test]
    fn agent_claude_md_contains_rules() {
        assert!(AGENT_CLAUDE_MD.contains("NEVER output source code"));
        assert!(AGENT_CLAUDE_MD.contains("cargo test"));
    }
}
