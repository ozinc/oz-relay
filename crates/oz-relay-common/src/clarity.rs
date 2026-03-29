// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Intent Clarity Gate — scores intents before burning tokens.
//!
//! EARS-inspired (Easy Approach to Requirements Specification).
//! Rejects vague intents with actionable feedback.
//! No LLM calls — pure heuristics. Runs in microseconds.
//!
//! Minimum score to proceed: 4 out of 10.
//! Below that: ERR_INTENT_UNCLEAR with specific guidance.

use crate::intent::Intent;
use serde::{Deserialize, Serialize};

/// Minimum clarity score required to proceed with a build.
pub const MIN_CLARITY_SCORE: i32 = 4;

/// Result of the clarity gate evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarityScore {
    /// Total score (max 10, min -5).
    pub score: i32,
    /// Whether the intent passes the gate.
    pub passes: bool,
    /// Breakdown of scoring signals.
    pub signals: Vec<ClaritySignal>,
    /// Actionable feedback if the intent doesn't pass.
    pub feedback: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaritySignal {
    pub name: String,
    pub points: i32,
    pub reason: String,
}

/// Vague words that indicate an unmeasurable intent.
const VAGUE_WORDS: &[&str] = &[
    "better", "improve", "optimize", "enhance", "faster", "cleaner",
    "more efficient", "refactor", "modernize", "upgrade", "polish",
    "fix everything", "make it work", "sort out",
];

/// Words that indicate a specific, testable behavior.
const SPECIFIC_WORDS: &[&str] = &[
    "return", "returns", "shall", "should", "must", "when", "where",
    "while", "given", "then", "expect", "produce", "output", "result",
    "null", "error", "throw", "emit", "yield",
];

/// Evaluate an intent's clarity. Returns a score and feedback.
pub fn evaluate_clarity(intent: &Intent) -> ClarityScore {
    let mut signals = Vec::new();
    let mut feedback = Vec::new();

    let desc_lower = intent.description.to_lowercase();
    let motivation_lower = intent.motivation.to_lowercase();

    // +2: Test case has a concrete query (not just prose)
    let has_concrete_query = intent.test_cases.iter().any(|tc| {
        let q = tc.query.to_uppercase();
        q.contains("MATCH") || q.contains("RETURN") || q.contains("CREATE")
            || q.contains("MERGE") || q.contains("CALL") || q.contains("SELECT")
            || q.contains("INSERT") || tc.query.contains('(') // function call
    });
    if has_concrete_query {
        signals.push(ClaritySignal {
            name: "concrete_query".into(),
            points: 2,
            reason: "Test case contains a concrete query".into(),
        });
    } else {
        signals.push(ClaritySignal {
            name: "concrete_query".into(),
            points: 0,
            reason: "Test case query is not a concrete operation".into(),
        });
        feedback.push(
            "Test query should be a concrete operation (e.g., RETURN upper('hello'), MATCH (n) WHERE ...)".into()
        );
    }

    // +2: Expected behavior is specific (uses specific words, has concrete output)
    let has_specific_expected = intent.test_cases.iter().any(|tc| {
        let exp_lower = tc.expected_behavior.to_lowercase();
        SPECIFIC_WORDS.iter().any(|w| exp_lower.contains(w))
            || tc.expected_behavior.contains('\'') // contains a literal value
            || tc.expected_behavior.contains('"')
    });
    if has_specific_expected {
        signals.push(ClaritySignal {
            name: "specific_expected".into(),
            points: 2,
            reason: "Expected behavior describes a specific outcome".into(),
        });
    } else {
        signals.push(ClaritySignal {
            name: "specific_expected".into(),
            points: 0,
            reason: "Expected behavior is not specific enough".into(),
        });
        feedback.push(
            "Expected behavior should describe a concrete result (e.g., \"Returns 'HELLO WORLD'\", \"Returns null when no edge exists\")".into()
        );
    }

    // +2: Description names a specific function, operator, or feature
    let names_specific = desc_lower.contains("function")
        || desc_lower.contains("operator")
        || desc_lower.contains("predicate")
        || desc_lower.contains("clause")
        || desc_lower.contains("keyword")
        || desc_lower.contains("statement")
        || desc_lower.contains("()")   // function reference
        || desc_lower.contains("match")
        || desc_lower.contains("return")
        || desc_lower.contains("where")
        || desc_lower.contains("index");
    if names_specific {
        signals.push(ClaritySignal {
            name: "specific_feature".into(),
            points: 2,
            reason: "Description names a specific feature or operation".into(),
        });
    } else {
        signals.push(ClaritySignal {
            name: "specific_feature".into(),
            points: 0,
            reason: "Description doesn't name a specific feature".into(),
        });
        feedback.push(
            "Description should name a specific function, operator, clause, or feature".into()
        );
    }

    // +1: Motivation explains a real use case (> 20 chars, not just repeating description)
    let has_real_motivation = intent.motivation.len() > 20
        && !motivation_lower.contains(&desc_lower[..desc_lower.len().min(20)]);
    if has_real_motivation {
        signals.push(ClaritySignal {
            name: "real_motivation".into(),
            points: 1,
            reason: "Motivation explains a real use case".into(),
        });
    } else {
        signals.push(ClaritySignal {
            name: "real_motivation".into(),
            points: 0,
            reason: "Motivation is too short or repeats the description".into(),
        });
    }

    // +1: Has input/setup data for test reproducibility
    let has_setup_data = intent.test_cases.iter().any(|tc| tc.input_data.is_some());
    if has_setup_data {
        signals.push(ClaritySignal {
            name: "setup_data".into(),
            points: 1,
            reason: "Test case includes setup data".into(),
        });
    }

    // -3: Long description that says nothing specific (padding)
    if intent.description.len() > 200 && !names_specific {
        signals.push(ClaritySignal {
            name: "padding".into(),
            points: -3,
            reason: "Long description without specific feature names".into(),
        });
        feedback.push("Description is long but doesn't name a specific feature. Be concise and specific.".into());
    }

    // -2: No concrete expected output
    let all_vague_expected = intent.test_cases.iter().all(|tc| {
        let exp_lower = tc.expected_behavior.to_lowercase();
        exp_lower == "works" || exp_lower == "correct" || exp_lower == "should work"
            || exp_lower == "passes" || exp_lower == "no error"
            || exp_lower.len() < 10
    });
    if all_vague_expected && !intent.test_cases.is_empty() {
        signals.push(ClaritySignal {
            name: "vague_expected".into(),
            points: -2,
            reason: "Expected behavior is too vague to verify".into(),
        });
        feedback.push("Expected behavior like 'works' or 'correct' is untestable. Describe the specific output.".into());
    }

    // -2: Uses unmeasurable words without a metric
    let has_vague_words = VAGUE_WORDS.iter().any(|w| desc_lower.contains(w));
    if has_vague_words {
        signals.push(ClaritySignal {
            name: "vague_words".into(),
            points: -2,
            reason: "Description uses unmeasurable words (better, improve, optimize...)".into(),
        });
        feedback.push(
            "Words like 'better', 'improve', 'optimize' need a measurable target. What specific behavior should change?".into()
        );
    }

    let score: i32 = signals.iter().map(|s| s.points).sum();
    let passes = score >= MIN_CLARITY_SCORE;

    if !passes && feedback.is_empty() {
        feedback.push(format!(
            "Intent scored {}/10 (minimum {}). Add concrete test queries and specific expected outputs.",
            score, MIN_CLARITY_SCORE
        ));
    }

    ClarityScore {
        score,
        passes,
        signals,
        feedback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::{IntentCategory, IntentContext, TestCase};

    fn make_intent(desc: &str, motivation: &str, query: &str, expected: &str) -> Intent {
        Intent {
            description: desc.into(),
            motivation: motivation.into(),
            category: IntentCategory::Feature,
            test_cases: vec![TestCase {
                query: query.into(),
                expected_behavior: expected.into(),
                input_data: None,
            }],
            context: IntentContext {
                arcflow_version: "1.7.0".into(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn clear_intent_passes() {
        let intent = make_intent(
            "Add a built-in function upper() that converts a string to uppercase",
            "Need to normalize string values for case-insensitive comparison",
            "RETURN upper('hello world')",
            "Returns 'HELLO WORLD'",
        );
        let score = evaluate_clarity(&intent);
        assert!(score.passes, "score={} signals={:?}", score.score, score.signals);
        assert!(score.score >= MIN_CLARITY_SCORE);
    }

    #[test]
    fn vague_intent_rejected() {
        let intent = make_intent(
            "Make it better",
            "It should be improved",
            "run the thing",
            "works",
        );
        let score = evaluate_clarity(&intent);
        assert!(!score.passes, "score={} should fail", score.score);
        assert!(!score.feedback.is_empty());
    }

    #[test]
    fn vague_expected_penalized() {
        let intent = make_intent(
            "Add OPTIONAL MATCH clause support",
            "Need optional relationship traversal for graph queries",
            "OPTIONAL MATCH (a)-[:KNOWS]->(b) RETURN a, b",
            "works",
        );
        let score = evaluate_clarity(&intent);
        // Has concrete query (+2), specific feature (+2), real motivation (+1) = 5
        // But vague expected (-2) = 3, should fail
        assert!(!score.passes, "score={} vague expected should fail", score.score);
    }

    #[test]
    fn optimize_without_metric_penalized() {
        let intent = make_intent(
            "Optimize the query engine to be faster",
            "Performance needs to be better",
            "MATCH (n) RETURN n",
            "Returns nodes faster",
        );
        let score = evaluate_clarity(&intent);
        assert!(
            score.signals.iter().any(|s| s.name == "vague_words"),
            "should flag vague words"
        );
    }

    #[test]
    fn feedback_is_actionable() {
        let intent = make_intent(
            "Fix the bug",
            "It's broken",
            "do the thing",
            "correct",
        );
        let score = evaluate_clarity(&intent);
        assert!(!score.passes);
        // Should have multiple actionable feedback items
        assert!(score.feedback.len() >= 2, "feedback: {:?}", score.feedback);
        // Feedback should tell them what to do, not just what's wrong
        assert!(
            score.feedback.iter().any(|f| f.contains("should") || f.contains("e.g.")),
            "feedback should be actionable: {:?}",
            score.feedback
        );
    }

    #[test]
    fn ears_pattern_passes() {
        let intent = make_intent(
            "Where a MATCH clause references a non-existent label, the system shall return an empty result set",
            "Currently throws an error instead of returning empty results, breaking downstream pipelines",
            "MATCH (n:NonExistentLabel) RETURN n",
            "Returns an empty result set with zero rows",
        );
        let score = evaluate_clarity(&intent);
        assert!(score.passes, "EARS pattern should pass: score={}", score.score);
    }

    #[test]
    fn score_breakdown_visible() {
        let intent = make_intent(
            "Add upper() function",
            "Need case normalization for string comparison use cases",
            "RETURN upper('hello')",
            "Returns 'HELLO'",
        );
        let score = evaluate_clarity(&intent);
        assert!(!score.signals.is_empty());
        // Each signal should have a name and reason
        for signal in &score.signals {
            assert!(!signal.name.is_empty());
            assert!(!signal.reason.is_empty());
        }
    }
}
