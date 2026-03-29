// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! A2A (Agent2Agent) protocol types — Rust representations of the Google A2A
//! specification primitives used by OIP.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// AgentCard — capability advertisement (served at /.well-known/agent.json)
// ---------------------------------------------------------------------------

/// An A2A AgentCard declares a remote agent's identity, capabilities, and
/// authentication requirements. Served at `/.well-known/agent.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub url: String,
    pub version: String,
    pub capabilities: AgentCapabilities,
    pub authentication: AuthenticationInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub streaming: bool,
    pub push_notifications: bool,
    pub intent_categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationInfo {
    pub schemes: Vec<String>,
    pub credentials_url: Option<String>,
}

impl AgentCard {
    /// Build the default OZ Relay AgentCard for a given product.
    pub fn for_product(url: &str, product: &str) -> Self {
        Self {
            name: format!("oz-relay-{}", product),
            description: format!(
                "OZ Intent-Source Relay — submit change intents to {}, \
                 receive compiled artifacts without source access",
                product
            ),
            url: url.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            capabilities: AgentCapabilities {
                streaming: true,
                push_notifications: false,
                intent_categories: vec![
                    "bug-fix".into(),
                    "feature".into(),
                    "performance".into(),
                    "compatibility".into(),
                ],
            },
            authentication: AuthenticationInfo {
                schemes: vec!["bearer".into()],
                credentials_url: Some("https://oz.global/relay/keys".into()),
            },
            documentation_url: Some(
                "https://github.com/ozinc/oz-relay".into(),
            ),
        }
    }

    /// Build the ArcFlow relay AgentCard (convenience for the default product).
    pub fn arcflow_relay(url: &str) -> Self {
        Self::for_product(url, "arcflow")
    }
}

// ---------------------------------------------------------------------------
// Task — lifecycle-managed unit of work
// ---------------------------------------------------------------------------

/// A2A Task state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Failed,
    Rejected,
    Canceled,
}

impl TaskState {
    /// Returns true if this is a terminal state (no further transitions).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskState::Completed | TaskState::Failed | TaskState::Rejected | TaskState::Canceled
        )
    }

    /// Validate a state transition. Returns `Ok(next)` if valid.
    pub fn transition(self, next: TaskState) -> Result<TaskState, TaskTransitionError> {
        if self.is_terminal() {
            return Err(TaskTransitionError {
                from: self,
                to: next,
            });
        }
        let valid = match (self, next) {
            // From submitted
            (TaskState::Submitted, TaskState::Working) => true,
            (TaskState::Submitted, TaskState::Rejected) => true,
            (TaskState::Submitted, TaskState::Canceled) => true,
            // From working
            (TaskState::Working, TaskState::Completed) => true,
            (TaskState::Working, TaskState::Failed) => true,
            (TaskState::Working, TaskState::InputRequired) => true,
            (TaskState::Working, TaskState::Canceled) => true,
            // From input_required
            (TaskState::InputRequired, TaskState::Working) => true,
            (TaskState::InputRequired, TaskState::Canceled) => true,
            _ => false,
        };
        if valid {
            Ok(next)
        } else {
            Err(TaskTransitionError {
                from: self,
                to: next,
            })
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskTransitionError {
    pub from: TaskState,
    pub to: TaskState,
}

impl std::fmt::Display for TaskTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid task state transition: {:?} → {:?}",
            self.from, self.to
        )
    }
}

impl std::error::Error for TaskTransitionError {}

/// A2A Task with OIP extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: Uuid,
    /// Developer who submitted this task (JWT `sub` claim).
    pub owner: String,
    pub state: TaskState,
    pub messages: Vec<Message>,
    pub artifacts: Vec<Artifact>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
}

impl Task {
    pub fn new(owner: &str, intent_message: Message) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            owner: owner.to_string(),
            state: TaskState::Submitted,
            messages: vec![intent_message],
            artifacts: vec![],
            created_at: now,
            updated_at: now,
            status_message: None,
        }
    }

    /// Transition the task to a new state, updating the timestamp.
    pub fn transition(&mut self, next: TaskState) -> Result<(), TaskTransitionError> {
        let validated = self.state.transition(next)?;
        self.state = validated;
        self.updated_at = Utc::now();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Message — one communication turn
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Agent,
}

/// A2A Message — one turn of communication. For OIP, the `user` role carries
/// the developer's Intent, and the `agent` role carries the server's filtered
/// behavioral summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub role: MessageRole,
    pub parts: Vec<Part>,
}

/// A2A Part — one unit of content within a Message or Artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Part {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "data")]
    Data {
        #[serde(rename = "mimeType")]
        mime_type: String,
        data: serde_json::Value,
    },
    #[serde(rename = "binary")]
    Binary {
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(rename = "base64Data")]
        base64_data: String,
    },
}

// ---------------------------------------------------------------------------
// Artifact — output binary with signed manifest
// ---------------------------------------------------------------------------

/// A2A Artifact — output of a completed Task. For OIP, contains the compiled
/// binary and a signed manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub artifact_id: Uuid,
    pub name: String,
    pub parts: Vec<Part>,
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 envelope (A2A transport)
// ---------------------------------------------------------------------------

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// JSON-RPC 2.0 response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    pub fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: serde_json::Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

// A2A standard JSON-RPC error codes
pub const ERR_TASK_NOT_FOUND: i32 = -32001;
pub const ERR_TASK_NOT_CANCELABLE: i32 = -32002;
pub const ERR_INVALID_INTENT: i32 = -32003;
pub const ERR_RATE_LIMITED: i32 = -32004;
pub const ERR_UNAUTHORIZED: i32 = -32005;
pub const ERR_KEY_NOT_ACTIVE: i32 = -32006;
