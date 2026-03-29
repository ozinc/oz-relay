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
    /// The original report (query sanitized for privacy).
    pub report: BugReport,
    /// When the report was first received.
    pub received_at: DateTime<Utc>,
    /// When the most recent duplicate was received.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Deduplication fingerprint: hash of (error_message + version + query_structure).
    pub fingerprint: String,
    /// Number of times this exact bug has been reported.
    pub occurrences: u32,
    /// Current status.
    pub status: String,
}

/// Generate a deduplication fingerprint from a bug report.
/// Hashes error_message + arcflow_version + query structure (without literal values).
pub fn fingerprint(report: &BugReport) -> String {
    let query_structure = report
        .query
        .as_deref()
        .map(strip_query_literals)
        .unwrap_or_default();
    let input = format!(
        "{}|{}|{}",
        report.error_message.trim().to_lowercase(),
        report.arcflow_version,
        query_structure
    );
    format!("{:016x}", fnv_hash(input.as_bytes()))
}

/// Strip literal values from a query for privacy and deduplication.
/// "CREATE (n:Person {name: 'Alice', ssn: '123-45-6789'})" →
/// "CREATE (n:Person {name: ?, ssn: ?})"
pub fn sanitize_query(query: &str) -> String {
    let mut result = String::with_capacity(query.len());
    let mut in_string = false;
    let mut quote_char = ' ';

    let mut chars = query.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            if c == quote_char && chars.peek() != Some(&quote_char) {
                result.push('?');
                result.push(c);
                in_string = false;
            }
            // Skip string contents
        } else if c == '\'' || c == '"' {
            quote_char = c;
            in_string = true;
            result.push(c);
        } else {
            result.push(c);
        }
    }
    result
}

/// Strip literals for fingerprinting (more aggressive than sanitize_query).
fn strip_query_literals(query: &str) -> String {
    let sanitized = sanitize_query(query);
    // Also normalize whitespace
    sanitized.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn fnv_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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

/// Words that indicate a specific, testable behavior (from clarity.rs EARS patterns).
const SPECIFIC_WORDS: &[&str] = &[
    "return", "returns", "shall", "should", "must", "when", "where",
    "while", "given", "then", "expect", "produce", "output", "result",
    "null", "error", "throw", "emit", "yield",
];

/// Words that indicate a concrete query (from clarity.rs).
const QUERY_KEYWORDS: &[&str] = &[
    "MATCH", "RETURN", "CREATE", "MERGE", "CALL", "SELECT", "INSERT",
];

/// Result of triaging a bug report.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriageResult {
    /// Whether the bug report has enough clarity to auto-generate an Intent.
    pub can_convert: bool,
    /// The generated intent, if the report was clear enough.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_intent: Option<Intent>,
    /// Fields or information that are missing or too vague.
    pub needs_info: Vec<String>,
}

/// Triage a stored bug report using EARS clarity heuristics.
///
/// If the report has enough structure (error message, query, specific language),
/// generates an Intent automatically. Otherwise returns actionable feedback
/// about what information is missing.
pub fn triage_bug(stored: &StoredBugReport) -> TriageResult {
    let mut needs_info = Vec::new();
    let report = &stored.report;

    let error_lower = report.error_message.to_lowercase();

    // Check 1: error_message must contain specific/actionable language
    let has_specific_error = SPECIFIC_WORDS.iter().any(|w| error_lower.contains(w))
        || error_lower.contains("not supported")
        || error_lower.contains("not implemented")
        || error_lower.contains("unexpected")
        || error_lower.contains("failed")
        || error_lower.contains("invalid")
        || error_lower.contains("panic");

    if !has_specific_error {
        needs_info.push(
            "error_message should describe a specific failure (e.g., 'returns null instead of error', 'panics when ...')".into()
        );
    }

    // Check 2: must have a query to use as test case
    let has_query = report.query.as_ref().is_some_and(|q| {
        let upper = q.to_uppercase();
        QUERY_KEYWORDS.iter().any(|kw| upper.contains(kw)) || q.contains('(')
    });

    if !has_query {
        needs_info.push(
            "query field is required with a concrete operation (e.g., MATCH, RETURN, CREATE)".into()
        );
    }

    // Check 3: error_message should be long enough to be meaningful
    if report.error_message.trim().len() < 15 {
        needs_info.push(
            "error_message is too short to be actionable — describe the specific error".into()
        );
    }

    // Require at least specific error + concrete query to auto-convert
    let can_convert = has_specific_error && has_query && report.error_message.trim().len() >= 15;

    let generated_intent = if can_convert {
        let query = report.query.clone().unwrap_or_default();
        Some(Intent {
            description: report.error_message.clone(),
            motivation: format!(
                "Bug report {}: {} (version {})",
                stored.id, report.category, report.arcflow_version
            ),
            category: IntentCategory::BugFix,
            test_cases: vec![TestCase {
                query: query.clone(),
                expected_behavior: format!(
                    "Should not produce error: {}",
                    truncate(&report.error_message, 200)
                ),
                input_data: report.context.clone(),
            }],
            context: IntentContext {
                error_logs: Some(report.error_message.clone()),
                stack_trace: report.stack_trace.clone(),
                reproduction_steps: Some(format!("Execute query: {}", query)),
                arcflow_version: report.arcflow_version.clone(),
                target_triple: report.target_triple.clone(),
            },
        })
    } else {
        None
    };

    TriageResult {
        can_convert,
        generated_intent,
        needs_info,
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_literals() {
        let q = "CREATE (n:Person {name: 'Alice', ssn: '123-45-6789'}) RETURN n";
        let s = sanitize_query(q);
        assert!(!s.contains("Alice"));
        assert!(!s.contains("123-45-6789"));
        assert!(s.contains("name: '?'"));
        assert!(s.contains("ssn: '?'"));
        assert!(s.contains("CREATE"));
        assert!(s.contains("RETURN"));
    }

    #[test]
    fn sanitize_preserves_structure() {
        let q = "MATCH (a)-[:KNOWS]->(b) WHERE a.age > 30 RETURN b.name";
        let s = sanitize_query(q);
        assert_eq!(s, q); // no string literals to strip
    }

    #[test]
    fn fingerprint_deduplicates() {
        let r1 = BugReport {
            error_message: "QueryError: += not supported".into(),
            arcflow_version: "1.7.0".into(),
            category: "runtime-error".into(),
            stack_trace: None,
            query: Some("SET n.count += 1".into()),
            trace_id: None,
            target_triple: None,
            context: None,
        };
        let r2 = r1.clone(); // identical
        assert_eq!(fingerprint(&r1), fingerprint(&r2));

        // Different error → different fingerprint
        let mut r3 = r1.clone();
        r3.error_message = "QueryError: -= not supported".into();
        assert_ne!(fingerprint(&r1), fingerprint(&r3));
    }

    #[test]
    fn fingerprint_ignores_literal_values() {
        let r1 = BugReport {
            error_message: "QueryError: invalid type".into(),
            arcflow_version: "1.7.0".into(),
            category: "runtime-error".into(),
            stack_trace: None,
            query: Some("CREATE (n {name: 'Alice'})".into()),
            trace_id: None,
            target_triple: None,
            context: None,
        };
        let mut r2 = r1.clone();
        r2.query = Some("CREATE (n {name: 'Bob'})".into());
        // Same structure, different literals → same fingerprint
        assert_eq!(fingerprint(&r1), fingerprint(&r2));
    }

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

    fn make_stored(report: BugReport) -> StoredBugReport {
        let fp = fingerprint(&report);
        StoredBugReport {
            id: "20260329-test01".into(),
            report,
            received_at: chrono::Utc::now(),
            last_seen_at: None,
            fingerprint: fp,
            occurrences: 1,
            status: "incoming".into(),
        }
    }

    #[test]
    fn triage_clear_bug_converts() {
        let stored = make_stored(BugReport {
            error_message: "QueryError: OPTIONAL not supported".into(),
            arcflow_version: "1.7.0".into(),
            category: "runtime-error".into(),
            stack_trace: Some("at wc_core::query::execute".into()),
            query: Some("OPTIONAL MATCH (a)-[:KNOWS]->(b) RETURN a".into()),
            trace_id: None,
            target_triple: None,
            context: None,
        });
        let result = triage_bug(&stored);
        assert!(result.can_convert);
        assert!(result.needs_info.is_empty());
        let intent = result.generated_intent.unwrap();
        assert_eq!(intent.category, IntentCategory::BugFix);
        assert!(intent.description.contains("OPTIONAL not supported"));
        assert!(!intent.test_cases.is_empty());
        assert!(intent.test_cases[0].query.contains("OPTIONAL MATCH"));
        assert!(intent.context.stack_trace.is_some());
    }

    #[test]
    fn triage_vague_bug_needs_info() {
        let stored = make_stored(BugReport {
            error_message: "it broke".into(),
            arcflow_version: "1.7.0".into(),
            category: "runtime-error".into(),
            stack_trace: None,
            query: None,
            trace_id: None,
            target_triple: None,
            context: None,
        });
        let result = triage_bug(&stored);
        assert!(!result.can_convert);
        assert!(result.generated_intent.is_none());
        assert!(!result.needs_info.is_empty());
    }

    #[test]
    fn triage_no_query_needs_info() {
        let stored = make_stored(BugReport {
            error_message: "QueryError: unexpected token near WHERE clause".into(),
            arcflow_version: "1.7.0".into(),
            category: "parse-error".into(),
            stack_trace: None,
            query: None,
            trace_id: None,
            target_triple: None,
            context: None,
        });
        let result = triage_bug(&stored);
        assert!(!result.can_convert);
        assert!(result.needs_info.iter().any(|s| s.contains("query")));
    }

    #[test]
    fn triage_short_error_needs_info() {
        let stored = make_stored(BugReport {
            error_message: "error".into(),
            arcflow_version: "1.7.0".into(),
            category: "runtime-error".into(),
            stack_trace: None,
            query: Some("MATCH (n) RETURN n".into()),
            trace_id: None,
            target_triple: None,
            context: None,
        });
        let result = triage_bug(&stored);
        assert!(!result.can_convert);
        assert!(result.needs_info.iter().any(|s| s.contains("too short")));
    }

    #[test]
    fn triage_intent_fields_populated() {
        let stored = make_stored(BugReport {
            error_message: "panics when processing null property values".into(),
            arcflow_version: "2.0.0".into(),
            category: "crash".into(),
            stack_trace: Some("thread 'main' panicked at...".into()),
            query: Some("CREATE (n {name: null}) RETURN n".into()),
            trace_id: Some("trace-abc".into()),
            target_triple: Some("x86_64-unknown-linux-gnu".into()),
            context: Some("Happens with any null property".into()),
        });
        let result = triage_bug(&stored);
        assert!(result.can_convert);
        let intent = result.generated_intent.unwrap();
        assert_eq!(intent.context.arcflow_version, "2.0.0");
        assert_eq!(intent.context.target_triple.as_deref(), Some("x86_64-unknown-linux-gnu"));
        assert!(intent.context.stack_trace.is_some());
        assert!(intent.motivation.contains("20260329-test01"));
        assert_eq!(intent.test_cases[0].input_data.as_deref(), Some("Happens with any null property"));
    }
}
