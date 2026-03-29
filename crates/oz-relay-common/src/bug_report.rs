// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Bug report schema — structured error reports from end users.
//! Routed to bugs/incoming/ for triage.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::intent::{Intent, IntentCategory, IntentContext, TestCase};

/// A structured bug report from an end user or OTEL exporter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BugReport {
    /// Error message from the ArcFlow runtime.
    pub error_message: String,
    /// ArcFlow version producing the error.
    pub arcflow_version: String,
    /// Error category.
    #[serde(default = "default_category")]
    pub category: String,
    /// Stack trace or panic output (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<String>,
    /// Query that triggered the error (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// OTEL trace ID for correlation (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// Target platform (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_triple: Option<String>,
    /// Additional context from the user (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

fn default_category() -> String {
    "runtime-error".into()
}

/// A bug report with server-assigned metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredBugReport {
    /// Server-assigned ID.
    pub id: String,
    /// The original report.
    pub report: BugReport,
    /// When the report was received.
    pub received_at: DateTime<Utc>,
    /// Source IP (hashed for privacy).
    pub source_hash: String,
    /// Current status.
    pub status: String,
}

/// Maximum lengths for bug report fields (prevent abuse).
pub const MAX_ERROR_MESSAGE_LEN: usize = 5000;
pub const MAX_STACK_TRACE_LEN: usize = 10_000;
pub const MAX_QUERY_LEN: usize = 2000;
pub const MAX_CONTEXT_LEN: usize = 2000;

/// Validate a bug report. Returns a list of errors.
pub fn validate_bug_report(report: &BugReport) -> Vec<String> {
    let mut errors = Vec::new();

    if report.error_message.trim().is_empty() {
        errors.push("error_message must not be empty".into());
    } else if report.error_message.len() > MAX_ERROR_MESSAGE_LEN {
        errors.push(format!("error_message exceeds {} characters", MAX_ERROR_MESSAGE_LEN));
    }

    if report.arcflow_version.trim().is_empty() {
        errors.push("arcflow_version must not be empty".into());
    }

    if let Some(ref trace) = report.stack_trace {
        if trace.len() > MAX_STACK_TRACE_LEN {
            errors.push(format!("stack_trace exceeds {} characters", MAX_STACK_TRACE_LEN));
        }
    }

    if let Some(ref query) = report.query {
        if query.len() > MAX_QUERY_LEN {
            errors.push(format!("query exceeds {} characters", MAX_QUERY_LEN));
        }
    }

    if let Some(ref ctx) = report.context {
        if ctx.len() > MAX_CONTEXT_LEN {
            errors.push(format!("context exceeds {} characters", MAX_CONTEXT_LEN));
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_report_passes() {
        let report = BugReport {
            error_message: "QueryError: OPTIONAL not supported".into(),
            arcflow_version: "1.7.0".into(),
            category: "runtime-error".into(),
            stack_trace: Some("at wc_core::query::execute".into()),
            query: Some("OPTIONAL MATCH (a)-[:KNOWS]->(b) RETURN a".into()),
            trace_id: Some("abc123".into()),
            target_triple: None,
            context: None,
        };
        assert!(validate_bug_report(&report).is_empty());
    }

    #[test]
    fn empty_error_rejected() {
        let report = BugReport {
            error_message: "".into(),
            arcflow_version: "1.7.0".into(),
            category: "runtime-error".into(),
            stack_trace: None,
            query: None,
            trace_id: None,
            target_triple: None,
            context: None,
        };
        assert!(!validate_bug_report(&report).is_empty());
    }

    #[test]
    fn oversized_fields_rejected() {
        let report = BugReport {
            error_message: "x".repeat(MAX_ERROR_MESSAGE_LEN + 1),
            arcflow_version: "1.7.0".into(),
            category: "runtime-error".into(),
            stack_trace: None,
            query: None,
            trace_id: None,
            target_triple: None,
            context: None,
        };
        let errors = validate_bug_report(&report);
        assert!(errors.iter().any(|e| e.contains("error_message")));
    }
}
