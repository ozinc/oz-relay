// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Build reports — structured responses to developers at each phase
//! of the intent lifecycle.

use serde::{Deserialize, Serialize};

/// Returned immediately on intent submission.
/// Confirms what the relay understood and gives the developer
/// a human-readable branch name to track their build.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarityReport {
    /// Human-readable branch name: relay/{developer}-{slug}-{short_id}
    pub branch: String,
    /// The relay's rephrased understanding of the intent.
    pub understood_as: String,
    /// Test criteria extracted from the intent.
    pub test_criteria: Vec<String>,
    /// Estimated build time in minutes.
    pub estimated_minutes: u32,
}

/// Returned when the build completes (success or failure).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildReport {
    /// Branch name in the relay repo.
    pub branch: String,
    /// Whether the build succeeded.
    pub success: bool,
    /// One-paragraph behavioral summary of what was built (no source details).
    pub summary: String,
    /// Test results.
    pub tests: TestReport,
    /// Cost and token usage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostReport>,
    /// Artifact info (only present on success).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostReport {
    /// Total tokens used by the claude session.
    pub total_tokens: u64,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Estimated cost in USD.
    pub cost_usd: f64,
    /// Build elapsed time in seconds.
    pub elapsed_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactReport {
    /// Artifact filename: arcflow-{developer}-{slug}-{short_id}.so
    pub name: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// SHA-256 hash.
    pub sha256: String,
    /// Whether the Ed25519 signature is present.
    pub signed: bool,
    /// Target triple.
    pub target_triple: String,
}

/// Generate a URL-safe slug from intent description.
/// "Add a built-in function upper()" → "upper-fn"
pub fn slugify(description: &str) -> String {
    let stop_words = [
        "a", "an", "the", "to", "for", "in", "on", "of", "and", "or",
        "that", "which", "with", "from", "add", "implement", "create",
        "built", "in", "support", "new", "function", "method",
    ];

    let lower = description.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && w.len() > 1 && !stop_words.contains(w))
        .take(3)
        .collect();

    if words.is_empty() {
        "intent".into()
    } else {
        words.join("-")
    }
}

/// Generate the branch name for a relay build.
/// Format: relay/{developer}-{slug}-{short_id}
pub fn branch_name(developer: &str, description: &str, task_id: &str) -> String {
    let slug = slugify(description);
    let short_id = &task_id[..8.min(task_id.len())];
    format!("relay/{}-{}-{}", developer, slug, short_id)
}

/// Generate the artifact filename.
/// Format: arcflow-{developer}-{slug}-{short_id}.{ext}
pub fn artifact_name(developer: &str, description: &str, task_id: &str, target: &str) -> String {
    let slug = slugify(description);
    let short_id = &task_id[..8.min(task_id.len())];
    let ext = if target.contains("darwin") {
        "dylib"
    } else if target.contains("wasm") {
        "wasm"
    } else {
        "so"
    };
    format!("arcflow-{}-{}-{}.{}", developer, slug, short_id, ext)
}

/// Build a clarity report from an intent.
pub fn clarity_report(
    developer: &str,
    description: &str,
    task_id: &str,
    test_cases: &[(String, String)], // (query, expected)
) -> ClarityReport {
    let branch = branch_name(developer, description, task_id);

    let test_criteria: Vec<String> = test_cases
        .iter()
        .map(|(query, expected)| format!("{} → {}", query, expected))
        .collect();

    ClarityReport {
        branch,
        understood_as: description.to_string(),
        test_criteria,
        estimated_minutes: 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(
            slugify("Add a built-in function upper() that converts a string to uppercase"),
            "upper-converts-string"
        );
    }

    #[test]
    fn slugify_short() {
        assert_eq!(slugify("Fix bug"), "fix-bug");
    }

    #[test]
    fn slugify_empty() {
        assert_eq!(slugify(""), "intent");
    }

    #[test]
    fn branch_name_format() {
        let branch = branch_name("dev_gudjon", "Add upper() function", "11d78823-cf99-41b9");
        assert_eq!(branch, "relay/dev_gudjon-upper-11d78823");
    }

    #[test]
    fn artifact_name_linux() {
        let name = artifact_name(
            "dev_gudjon",
            "Add upper() function",
            "11d78823-cf99-41b9",
            "x86_64-unknown-linux-gnu",
        );
        assert_eq!(name, "arcflow-dev_gudjon-upper-11d78823.so");
    }

    #[test]
    fn artifact_name_macos() {
        let name = artifact_name(
            "dev_alice",
            "Fix OPTIONAL MATCH",
            "a3f8b7c2-1234",
            "aarch64-apple-darwin",
        );
        assert_eq!(name, "arcflow-dev_alice-fix-optional-match-a3f8b7c2.dylib");
    }

    #[test]
    fn clarity_report_basic() {
        let report = clarity_report(
            "dev_gudjon",
            "Add upper() function",
            "11d78823-cf99",
            &[("RETURN upper('hello')" .into(), "Returns 'HELLO'".into())],
        );
        assert_eq!(report.branch, "relay/dev_gudjon-upper-11d78823");
        assert_eq!(report.test_criteria.len(), 1);
        assert_eq!(report.estimated_minutes, 10);
    }
}
