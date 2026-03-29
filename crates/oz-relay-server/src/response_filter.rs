// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Response filter — defense-in-depth layer that strips source code details
//! from agent output before it reaches the developer.
//!
//! The agent's CLAUDE.md is the primary boundary. This filter catches
//! anything that leaks through. It is deliberately aggressive — false
//! positives (removing safe content) are acceptable; false negatives
//! (leaking source) are not.

use regex::Regex;
use std::sync::LazyLock;

/// File path patterns.
static PATH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:crates/|src/|target/|tests/|benches/|examples/|\.rs\b|\.toml\b|Cargo\.lock|mod\.rs|lib\.rs|main\.rs|build\.rs)").unwrap()
});

/// Any code block (fenced with backticks).
static CODE_BLOCK_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)```[a-z]*\s*\n.*?```").unwrap()
});

/// Internal module/function references — ArcFlow-specific identifiers.
static INTERNAL_REF_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:wc_core|wc_types|wc_storage|wc_runtime|wc_query_ir|wc_query_compiler|wc_ffi|wc_sdk|wc_mcp|GraphStore|PropertyValue|ZSetOp|WalOp|DeltaEngine|StandingQuery|ArrangementStore|EGraphOptimizer|QueryPlan|PlanNode|grb_mxv|grb_mxm)\b").unwrap()
});

/// Rust syntax patterns that indicate source code even outside code blocks.
static RUST_SYNTAX_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|\s)(?:fn\s+\w+|pub\s+(?:fn|struct|enum|trait|mod|use|impl)|impl\s+\w+|#\[(?:derive|cfg|test|allow|inline)|use\s+(?:std|crate|super|self)::|mod\s+\w+\s*\{|let\s+(?:mut\s+)?\w+\s*[=:])").unwrap()
});

/// Line numbers pattern (e.g., "line 42", "L42", ":42:")
static LINE_NUMBER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:line\s+\d+|:\d+:\d+|L\d+(?:-L?\d+)?|\blines?\s+\d+(?:-\d+)?)").unwrap()
});

/// Base64-encoded content (long base64 strings that could contain encoded source).
static BASE64_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9+/]{60,}={0,2}").unwrap()
});

/// Maximum allowed response length (characters). Truncate beyond this
/// to prevent exfiltration via verbose output.
const MAX_RESPONSE_LEN: usize = 2000;

/// Filter agent output to remove source code details.
///
/// Deliberately aggressive. The developer should only receive:
/// - Whether the change was implemented
/// - Test pass/fail counts
/// - A one-sentence behavioral summary
pub fn filter_response(raw: &str) -> String {
    let mut filtered = raw.to_string();

    // 1. Remove all code blocks (any language)
    filtered = CODE_BLOCK_PATTERN
        .replace_all(&filtered, "[code removed]")
        .into_owned();

    // 2. Remove lines with file paths
    filtered = filtered
        .lines()
        .filter(|line| !PATH_PATTERN.is_match(line))
        .collect::<Vec<_>>()
        .join("\n");

    // 3. Remove lines with Rust syntax
    filtered = filtered
        .lines()
        .filter(|line| !RUST_SYNTAX_PATTERN.is_match(line))
        .collect::<Vec<_>>()
        .join("\n");

    // 4. Replace internal identifiers
    filtered = INTERNAL_REF_PATTERN
        .replace_all(&filtered, "[internal]")
        .into_owned();

    // 5. Remove line number references
    filtered = LINE_NUMBER_PATTERN
        .replace_all(&filtered, "[ref removed]")
        .into_owned();

    // 6. Remove base64 blobs
    filtered = BASE64_PATTERN
        .replace_all(&filtered, "[data removed]")
        .into_owned();

    // 7. Clean up excessive blank lines
    while filtered.contains("\n\n\n") {
        filtered = filtered.replace("\n\n\n", "\n\n");
    }

    // 8. Truncate to max length
    let mut result = filtered.trim().to_string();
    if result.len() > MAX_RESPONSE_LEN {
        result.truncate(MAX_RESPONSE_LEN);
        result.push_str("\n[response truncated]");
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_file_paths() {
        let input = "I modified crates/wc-core/src/lib.rs to add the feature.\nThe change works.";
        let filtered = filter_response(input);
        assert!(!filtered.contains("crates/"));
        assert!(!filtered.contains("lib.rs"));
        assert!(filtered.contains("The change works"));
    }

    #[test]
    fn filters_code_blocks() {
        let input = "I added a new function:\n```rust\nfn optional_match() {\n    // impl\n}\n```\nIt passes tests.";
        let filtered = filter_response(input);
        assert!(!filtered.contains("fn optional_match"));
        assert!(filtered.contains("[code removed]"));
        assert!(filtered.contains("It passes tests"));
    }

    #[test]
    fn filters_internal_references() {
        let input = "The GraphStore now supports optional match via ZSetOp::LeftJoin.";
        let filtered = filter_response(input);
        assert!(!filtered.contains("GraphStore"));
        assert!(!filtered.contains("ZSetOp"));
        assert!(filtered.contains("[internal]"));
    }

    #[test]
    fn preserves_clean_text() {
        let input = "OPTIONAL MATCH now returns null rows for missing paths. All 42 tests pass.";
        let filtered = filter_response(input);
        assert_eq!(filtered, input);
    }

    #[test]
    fn filters_toml_references() {
        let input = "Updated Cargo.toml with the new dependency.\nThe feature works.";
        let filtered = filter_response(input);
        assert!(!filtered.contains("Cargo.toml"));
        assert!(filtered.contains("The feature works"));
    }

    #[test]
    fn handles_multiple_code_blocks() {
        let input = "First:\n```\nlet x = 1;\n```\nThen:\n```rust\nlet y = 2;\n```\nDone.";
        let filtered = filter_response(input);
        assert!(!filtered.contains("let x"));
        assert!(!filtered.contains("let y"));
        assert!(filtered.contains("Done"));
    }

    #[test]
    fn filters_rust_syntax_outside_blocks() {
        let input = "I added pub fn new_feature() to handle this case.\nIt works now.";
        let filtered = filter_response(input);
        assert!(!filtered.contains("pub fn"));
        assert!(filtered.contains("It works now"));
    }

    #[test]
    fn filters_line_numbers() {
        let input = "The error was at line 847 in the query compiler.\nFixed now.";
        let filtered = filter_response(input);
        assert!(!filtered.contains("line 847"));
        assert!(filtered.contains("Fixed now"));
    }

    #[test]
    fn filters_base64_encoded_content() {
        let b64 = "a]W1wbCBHcmFwaFN0b3JlIHsKICAgIHB1YiBmbiBuZXcoKSAtPiBTZWxmIHsKICAgICAgICBTZWxmIHt9Cg==";
        let input = format!("Here is the data: {}\nDone.", b64);
        let filtered = filter_response(&input);
        assert!(!filtered.contains(b64));
        assert!(filtered.contains("[data removed]"));
    }

    #[test]
    fn truncates_long_responses() {
        // Use words separated by spaces to survive blank-line cleanup
        let long = "word ".repeat(600); // ~3000 chars
        let filtered = filter_response(&long);
        assert!(filtered.len() <= MAX_RESPONSE_LEN + 25); // +25 for truncation message
        assert!(filtered.contains("[response truncated]"));
    }

    #[test]
    fn filters_extended_internal_refs() {
        let input = "The DeltaEngine processes changes through StandingQuery evaluation.";
        let filtered = filter_response(input);
        assert!(!filtered.contains("DeltaEngine"));
        assert!(!filtered.contains("StandingQuery"));
    }
}
