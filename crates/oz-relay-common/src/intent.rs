// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! OIP Intent Schema — structured change specification for agent-mediated
//! contribution. Rides as a `data` Part inside an A2A Message.

use serde::{Deserialize, Serialize};

use crate::a2a::{Message, MessageRole, Part};

/// The category of change being requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentCategory {
    BugFix,
    Feature,
    Performance,
    Compatibility,
}

impl std::fmt::Display for IntentCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntentCategory::BugFix => write!(f, "bug_fix"),
            IntentCategory::Feature => write!(f, "feature"),
            IntentCategory::Performance => write!(f, "performance"),
            IntentCategory::Compatibility => write!(f, "compatibility"),
        }
    }
}

impl std::str::FromStr for IntentCategory {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "bug_fix" | "bug-fix" | "bugfix" => Ok(IntentCategory::BugFix),
            "feature" => Ok(IntentCategory::Feature),
            "performance" | "perf" => Ok(IntentCategory::Performance),
            "compatibility" | "compat" => Ok(IntentCategory::Compatibility),
            _ => Err(format!(
                "unknown category '{}': expected bug-fix, feature, performance, or compatibility",
                s
            )),
        }
    }
}

/// A single test case that defines expected behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    /// A query or operation to execute against the product.
    pub query: String,
    /// Human-readable description of the expected result.
    pub expected_behavior: String,
    /// Optional input data needed for the test (e.g., seed data).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_data: Option<String>,
}

/// Context supporting the intent — helps the server-side agent understand
/// the problem without back-and-forth.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntentContext {
    /// Error logs or messages encountered by the developer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_logs: Option<String>,
    /// Stack trace or panic output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<String>,
    /// Steps to reproduce the issue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reproduction_steps: Option<String>,
    /// Product version the developer is using.
    pub arcflow_version: String,
    /// Target platform (e.g., "aarch64-apple-darwin").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_triple: Option<String>,
}

/// OIP Intent — the core schema for a change request. Transmitted as a
/// `data` Part with `mimeType: "application/vnd.oip.intent+json"` inside
/// an A2A Message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    /// What behavior should change or be added.
    pub description: String,
    /// Why the change is needed.
    pub motivation: String,
    /// The category of change.
    pub category: IntentCategory,
    /// Test cases defining correct behavior (minimum 1).
    pub test_cases: Vec<TestCase>,
    /// Supporting context.
    pub context: IntentContext,
}

/// MIME type for OIP Intents carried as A2A data parts.
pub const INTENT_MIME_TYPE: &str = "application/vnd.oip.intent+json";

/// MIME type for OIP Artifact manifests.
pub const ARTIFACT_MANIFEST_MIME_TYPE: &str = "application/vnd.oip.artifact-manifest+json";

impl Intent {
    /// Wrap this Intent into an A2A Message with role: user.
    pub fn into_message(self) -> Message {
        let json = serde_json::to_value(&self).expect("Intent serialization cannot fail");
        Message {
            role: MessageRole::User,
            parts: vec![Part::Data {
                mime_type: INTENT_MIME_TYPE.into(),
                data: json,
            }],
        }
    }

    /// Extract an Intent from an A2A Message, if present.
    pub fn from_message(msg: &Message) -> Option<Intent> {
        for part in &msg.parts {
            if let Part::Data { mime_type, data } = part {
                if mime_type == INTENT_MIME_TYPE {
                    return serde_json::from_value(data.clone()).ok();
                }
            }
        }
        None
    }
}

/// Signed artifact manifest — delivered as a `data` Part alongside the binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactManifest {
    /// SHA-256 hash of the compiled artifact binary.
    pub sha256: String,
    /// Ed25519 signature of the SHA-256 hash.
    pub signature: String,
    /// ABI version of the product's FFI boundary.
    pub abi_version: String,
    /// Target triple (e.g., "aarch64-apple-darwin").
    pub target_triple: String,
    /// Build timestamp.
    pub timestamp: String,
    /// Product version that produced this artifact.
    pub arcflow_version: String,
}
