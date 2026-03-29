// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Response filter — strips source code details from agent output.
//!
//! Defense-in-depth layer. The primary boundary is the agent's CLAUDE.md
//! instructions which prohibit outputting source details. This filter
//! catches anything that leaks through.

use regex::Regex;
use std::sync::LazyLock;

/// Patterns that indicate source code leakage.
static PATH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:crates/|src/|target/|\.rs\b|\.toml\b|Cargo\.lock|mod\.rs)").unwrap()
});

/// Rust code block pattern.
static CODE_BLOCK_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)```(?:rust)?\s*\n.*?```").unwrap()
});

/// Internal module/function references.
static INTERNAL_REF_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:wc_core|wc_types|wc_storage|wc_runtime|wc_query_ir|wc_query_compiler|wc_ffi|wc_sdk|wc_mcp|GraphStore|PropertyValue|ZSetOp|WalOp)\b").unwrap()
});

/// Filter agent output to remove source code details.
pub fn filter_response(raw: &str) -> String {
    let mut filtered = raw.to_string();

    // Remove code blocks first (they may contain paths and identifiers)
    filtered = CODE_BLOCK_PATTERN
        .replace_all(&filtered, "[code removed]")
        .into_owned();

    // Remove lines containing file paths
    filtered = filtered
        .lines()
        .filter(|line| !PATH_PATTERN.is_match(line))
        .collect::<Vec<_>>()
        .join("\n");

    // Replace internal references with generic placeholders
    filtered = INTERNAL_REF_PATTERN
        .replace_all(&filtered, "[internal]")
        .into_owned();

    // Clean up excessive blank lines
    while filtered.contains("\n\n\n") {
        filtered = filtered.replace("\n\n\n", "\n\n");
    }

    filtered.trim().to_string()
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
}
