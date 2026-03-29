// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Intent validation — enforces schema constraints before submission.

use crate::intent::Intent;

/// Maximum length for the intent description field.
const MAX_DESCRIPTION_LEN: usize = 5000;
/// Maximum length for the motivation field.
const MAX_MOTIVATION_LEN: usize = 2000;
/// Maximum number of test cases per intent.
const MAX_TEST_CASES: usize = 20;
/// Maximum length for a single test case query.
const MAX_QUERY_LEN: usize = 2000;
/// Maximum length for error logs in context.
const MAX_CONTEXT_FIELD_LEN: usize = 10_000;

/// Validation error with a human-readable message.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for ValidationError {}

/// Validate an Intent, returning all errors found.
pub fn validate_intent(intent: &Intent) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // description: required, non-empty, bounded
    if intent.description.trim().is_empty() {
        errors.push(ValidationError {
            field: "description".into(),
            message: "must not be empty".into(),
        });
    } else if intent.description.len() > MAX_DESCRIPTION_LEN {
        errors.push(ValidationError {
            field: "description".into(),
            message: format!("exceeds maximum length of {} characters", MAX_DESCRIPTION_LEN),
        });
    }

    // motivation: required, non-empty, bounded
    if intent.motivation.trim().is_empty() {
        errors.push(ValidationError {
            field: "motivation".into(),
            message: "must not be empty".into(),
        });
    } else if intent.motivation.len() > MAX_MOTIVATION_LEN {
        errors.push(ValidationError {
            field: "motivation".into(),
            message: format!("exceeds maximum length of {} characters", MAX_MOTIVATION_LEN),
        });
    }

    // test_cases: at least one required
    if intent.test_cases.is_empty() {
        errors.push(ValidationError {
            field: "test_cases".into(),
            message: "at least one test case is required".into(),
        });
    } else if intent.test_cases.len() > MAX_TEST_CASES {
        errors.push(ValidationError {
            field: "test_cases".into(),
            message: format!("exceeds maximum of {} test cases", MAX_TEST_CASES),
        });
    }

    for (i, tc) in intent.test_cases.iter().enumerate() {
        if tc.query.trim().is_empty() {
            errors.push(ValidationError {
                field: format!("test_cases[{}].query", i),
                message: "must not be empty".into(),
            });
        } else if tc.query.len() > MAX_QUERY_LEN {
            errors.push(ValidationError {
                field: format!("test_cases[{}].query", i),
                message: format!("exceeds maximum length of {} characters", MAX_QUERY_LEN),
            });
        }
        if tc.expected_behavior.trim().is_empty() {
            errors.push(ValidationError {
                field: format!("test_cases[{}].expected_behavior", i),
                message: "must not be empty".into(),
            });
        }
    }

    // context.arcflow_version: required
    if intent.context.arcflow_version.trim().is_empty() {
        errors.push(ValidationError {
            field: "context.arcflow_version".into(),
            message: "must not be empty".into(),
        });
    }

    // context field length limits
    if let Some(ref logs) = intent.context.error_logs {
        if logs.len() > MAX_CONTEXT_FIELD_LEN {
            errors.push(ValidationError {
                field: "context.error_logs".into(),
                message: format!(
                    "exceeds maximum length of {} characters",
                    MAX_CONTEXT_FIELD_LEN
                ),
            });
        }
    }
    if let Some(ref trace) = intent.context.stack_trace {
        if trace.len() > MAX_CONTEXT_FIELD_LEN {
            errors.push(ValidationError {
                field: "context.stack_trace".into(),
                message: format!(
                    "exceeds maximum length of {} characters",
                    MAX_CONTEXT_FIELD_LEN
                ),
            });
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::{IntentCategory, IntentContext, TestCase};

    fn valid_intent() -> Intent {
        Intent {
            description: "Add OPTIONAL MATCH support".into(),
            motivation: "Need to query optional relationships that may not exist".into(),
            category: IntentCategory::Feature,
            test_cases: vec![TestCase {
                query: "OPTIONAL MATCH (a:Person)-[:KNOWS]->(b) RETURN a.name, b.name".into(),
                expected_behavior: "Returns b.name as null when no KNOWS relationship exists"
                    .into(),
                input_data: Some("CREATE (a:Person {name: 'Alice'})".into()),
            }],
            context: IntentContext {
                arcflow_version: "1.7.0".into(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn valid_intent_passes() {
        let errors = validate_intent(&valid_intent());
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn empty_description_rejected() {
        let mut intent = valid_intent();
        intent.description = "".into();
        let errors = validate_intent(&intent);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "description");
    }

    #[test]
    fn whitespace_description_rejected() {
        let mut intent = valid_intent();
        intent.description = "   ".into();
        let errors = validate_intent(&intent);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "description");
    }

    #[test]
    fn empty_motivation_rejected() {
        let mut intent = valid_intent();
        intent.motivation = "".into();
        let errors = validate_intent(&intent);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "motivation");
    }

    #[test]
    fn no_test_cases_rejected() {
        let mut intent = valid_intent();
        intent.test_cases.clear();
        let errors = validate_intent(&intent);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "test_cases");
    }

    #[test]
    fn too_many_test_cases_rejected() {
        let mut intent = valid_intent();
        intent.test_cases = (0..21)
            .map(|i| TestCase {
                query: format!("MATCH (n) WHERE n.id = {}", i),
                expected_behavior: "returns node".into(),
                input_data: None,
            })
            .collect();
        let errors = validate_intent(&intent);
        assert!(errors.iter().any(|e| e.field == "test_cases"));
    }

    #[test]
    fn empty_test_query_rejected() {
        let mut intent = valid_intent();
        intent.test_cases[0].query = "".into();
        let errors = validate_intent(&intent);
        assert!(errors.iter().any(|e| e.field == "test_cases[0].query"));
    }

    #[test]
    fn empty_expected_behavior_rejected() {
        let mut intent = valid_intent();
        intent.test_cases[0].expected_behavior = "".into();
        let errors = validate_intent(&intent);
        assert!(errors
            .iter()
            .any(|e| e.field == "test_cases[0].expected_behavior"));
    }

    #[test]
    fn empty_arcflow_version_rejected() {
        let mut intent = valid_intent();
        intent.context.arcflow_version = "".into();
        let errors = validate_intent(&intent);
        assert!(errors
            .iter()
            .any(|e| e.field == "context.arcflow_version"));
    }

    #[test]
    fn oversized_description_rejected() {
        let mut intent = valid_intent();
        intent.description = "x".repeat(5001);
        let errors = validate_intent(&intent);
        assert!(errors.iter().any(|e| e.field == "description"));
    }

    #[test]
    fn oversized_error_logs_rejected() {
        let mut intent = valid_intent();
        intent.context.error_logs = Some("x".repeat(10_001));
        let errors = validate_intent(&intent);
        assert!(errors.iter().any(|e| e.field == "context.error_logs"));
    }

    #[test]
    fn multiple_errors_reported() {
        let intent = Intent {
            description: "".into(),
            motivation: "".into(),
            category: IntentCategory::BugFix,
            test_cases: vec![],
            context: IntentContext {
                arcflow_version: "".into(),
                ..Default::default()
            },
        };
        let errors = validate_intent(&intent);
        assert!(errors.len() >= 4, "expected at least 4 errors, got {}", errors.len());
    }

    #[test]
    fn intent_message_roundtrip() {
        let intent = valid_intent();
        let msg = intent.clone().into_message();
        let recovered = Intent::from_message(&msg).expect("should recover intent from message");
        assert_eq!(recovered.description, intent.description);
        assert_eq!(recovered.category, intent.category);
        assert_eq!(recovered.test_cases.len(), intent.test_cases.len());
    }

    #[test]
    fn agent_card_serializes() {
        use crate::a2a::AgentCard;
        let card = AgentCard::arcflow_relay("https://relay.oz.global");
        let json = serde_json::to_string_pretty(&card).unwrap();
        assert!(json.contains("oz-relay-arcflow"));
        assert!(json.contains("bearer"));
        assert!(json.contains("bug-fix"));
        let back: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "oz-relay-arcflow");
    }

    #[test]
    fn task_state_transitions() {
        use crate::a2a::TaskState;
        assert!(TaskState::Submitted.transition(TaskState::Working).is_ok());
        assert!(TaskState::Submitted.transition(TaskState::Rejected).is_ok());
        assert!(TaskState::Working.transition(TaskState::Completed).is_ok());
        assert!(TaskState::Working.transition(TaskState::Failed).is_ok());
        assert!(TaskState::Working.transition(TaskState::InputRequired).is_ok());
        assert!(TaskState::InputRequired.transition(TaskState::Working).is_ok());
        assert!(TaskState::Submitted.transition(TaskState::Completed).is_err());
        assert!(TaskState::Completed.transition(TaskState::Working).is_err());
        assert!(TaskState::Failed.transition(TaskState::Working).is_err());
    }

    #[test]
    fn task_lifecycle() {
        use crate::a2a::Task;
        let intent = valid_intent();
        let mut task = Task::new("dev_test", intent.into_message());
        assert_eq!(task.state, crate::a2a::TaskState::Submitted);
        assert_eq!(task.owner, "dev_test");

        task.transition(crate::a2a::TaskState::Working).unwrap();
        assert_eq!(task.state, crate::a2a::TaskState::Working);

        task.transition(crate::a2a::TaskState::Completed).unwrap();
        assert_eq!(task.state, crate::a2a::TaskState::Completed);

        assert!(task.transition(crate::a2a::TaskState::Working).is_err());
    }

    #[test]
    fn json_rpc_response_success() {
        use crate::a2a::JsonRpcResponse;
        let resp = JsonRpcResponse::success(
            serde_json::json!(1),
            serde_json::json!({"state": "submitted"}),
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"submitted\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn json_rpc_response_error() {
        use crate::a2a::JsonRpcResponse;
        let resp = JsonRpcResponse::error(serde_json::json!(1), -32001, "task not found");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("-32001"));
        assert!(json.contains("task not found"));
        assert!(!json.contains("\"result\""));
    }
}
