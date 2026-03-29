// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! HTTP routes: A2A JSON-RPC endpoint, AgentCard, health check.
//!
//! Security fixes applied:
//! - #3: Request body size limit (1MB) via DefaultBodyLimit
//! - #5: Tenant isolation — task access filtered by JWT `sub` claim

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::middleware;
use axum::response::{IntoResponse, Json, Response, Sse};
use axum::routing::{get, post};
use axum::Router;

use oz_relay_common::a2a::*;
use oz_relay_common::intent::Intent;
use oz_relay_common::report;
use oz_relay_common::validation::validate_intent;

use crate::agent_bridge;
use crate::response_filter;
use crate::sandbox;

use crate::auth::{auth_middleware, RelayClaims};
use crate::AppState;

/// Build the full router.
pub fn build_router(state: Arc<AppState>) -> Router {
    // Public routes (no auth)
    let public = Router::new()
        .route("/.well-known/agent.json", get(agent_card))
        .route("/health", get(health));

    // Authenticated routes
    let authenticated = Router::new()
        .route("/a2a", post(a2a_endpoint))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .merge(public)
        .merge(authenticated)
        // FIX #3: 1MB request body size limit to prevent OOM DoS
        .layer(axum::extract::DefaultBodyLimit::max(1_048_576))
        .with_state(state)
}

/// GET /.well-known/agent.json
async fn agent_card(State(state): State<Arc<AppState>>) -> Json<AgentCard> {
    Json(AgentCard::arcflow_relay(&state.config.relay_url))
}

/// GET /health
async fn health() -> &'static str {
    "ok"
}

/// POST /a2a — JSON-RPC 2.0 dispatcher
async fn a2a_endpoint(
    State(state): State<Arc<AppState>>,
    claims: Option<axum::Extension<RelayClaims>>,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    // Validate JSON-RPC version
    if req.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            req.id,
            -32600,
            "invalid JSON-RPC version",
        ))
        .into_response();
    }

    let claims = match claims {
        Some(axum::Extension(c)) => c,
        None => {
            tracing::warn!(method = %req.method, "unauthenticated request");
            return Json(JsonRpcResponse::error(req.id, ERR_UNAUTHORIZED, "unauthorized"))
                .into_response()
        }
    };

    tracing::debug!(method = %req.method, sub = %claims.sub, tier = %claims.tier, status = %claims.status, "a2a request");

    match req.method.as_str() {
        "message/send" => handle_message_send(state, claims, req).await,
        "message/stream" => handle_message_stream(state, claims, req).await,
        "tasks/get" => handle_tasks_get(state, claims, req).await,
        "tasks/cancel" => handle_tasks_cancel(state, claims, req).await,
        _ => Json(JsonRpcResponse::error(
            req.id,
            -32601,
            format!("method not found: {}", req.method),
        ))
        .into_response(),
    }
}

/// message/send — submit an intent, create a task
async fn handle_message_send(
    state: Arc<AppState>,
    claims: RelayClaims,
    req: JsonRpcRequest,
) -> Response {
    // Entitlement check — reject before burning server-side tokens
    if !claims.is_active() {
        return Json(JsonRpcResponse::error(
            req.id,
            ERR_KEY_NOT_ACTIVE,
            format!(
                "key status is '{}' — only active keys can submit intents. \
                 apply for a Developer Relay Account at https://ozapi.net/relay/keys",
                claims.status
            ),
        ))
        .into_response();
    }

    // Rate limit check
    if let Err(retry_after) = state.rate_limiter.check(&claims.sub, &claims.tier) {
        return Json(JsonRpcResponse::error(
            req.id,
            ERR_RATE_LIMITED,
            format!("rate limit exceeded, retry after {} seconds", retry_after),
        ))
        .into_response();
    }

    // Parse the message from params
    let message: Message = match serde_json::from_value(req.params.clone()) {
        Ok(m) => m,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                req.id,
                -32602,
                format!("invalid message: {}", e),
            ))
            .into_response()
        }
    };

    // Validate the intent and extract it
    let intent = match Intent::from_message(&message) {
        Some(intent) => {
            let errors = validate_intent(&intent);
            if !errors.is_empty() {
                let error_msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
                return Json(JsonRpcResponse::error(
                    req.id,
                    ERR_INVALID_INTENT,
                    format!("intent validation failed: {}", error_msgs.join("; ")),
                ))
                .into_response();
            }
            intent
        }
        None => {
            return Json(JsonRpcResponse::error(
                req.id,
                ERR_INVALID_INTENT,
                "message must contain an intent (application/vnd.oip.intent+json)",
            ))
            .into_response();
        }
    };

    // Build clarity report — immediate feedback to developer
    let task_id_str = uuid::Uuid::new_v4().to_string();
    let branch = report::branch_name(&claims.sub, &intent.description, &task_id_str);
    let test_criteria: Vec<(String, String)> = intent
        .test_cases
        .iter()
        .map(|tc| (tc.query.clone(), tc.expected_behavior.clone()))
        .collect();
    let clarity = report::clarity_report(
        &claims.sub,
        &intent.description,
        &task_id_str,
        &test_criteria,
    );

    // Create the task with owner from JWT claims
    let task = state.task_manager.create_task(&claims.sub, message).await;
    let task_id = task.id;

    tracing::info!(
        task_id = %task_id,
        owner = %claims.sub,
        branch = %branch,
        category = %intent.category,
        description = %intent.description,
        "task created"
    );

    // Spawn the build session in the background if source repo is configured
    if let Some(ref source_repo) = state.config.source_repo {
        let source_repo = source_repo.clone();
        let timeout = Duration::from_secs(state.config.sandbox_timeout_secs);
        let task_manager = state.clone();
        let branch_name = branch.clone();
        let intent_desc = intent.description.clone();
        let developer = claims.sub.clone();

        tokio::spawn(async move {
            let start = std::time::Instant::now();

            // Transition to Working
            let _ = task_manager.task_manager.transition_task(task_id, TaskState::Working).await;
            tracing::info!(task_id = %task_id, branch = %branch_name, "build started");

            // Create worktree
            let session_id = task_id.to_string();
            let worktree = match sandbox::create_worktree(&source_repo, &session_id).await {
                Ok(w) => {
                    tracing::info!(task_id = %task_id, worktree = %w.display(), "worktree created");
                    w
                }
                Err(e) => {
                    tracing::error!(task_id = %task_id, error = %e, "worktree creation failed");
                    let _ = task_manager.task_manager.add_message(
                        task_id,
                        Message {
                            role: MessageRole::Agent,
                            parts: vec![Part::Text {
                                text: "Build setup failed.".into(),
                            }],
                        },
                    );
                    let _ = task_manager.task_manager.transition_task(task_id, TaskState::Failed).await;
                    return;
                }
            };

            // Prepare worktree with CLAUDE.md and prompt
            if let Err(e) = agent_bridge::prepare_worktree(&worktree, &intent).await {
                tracing::error!(task_id = %task_id, error = %e, "worktree preparation failed");
                let _ = task_manager.task_manager.add_message(
                    task_id,
                    Message {
                        role: MessageRole::Agent,
                        parts: vec![Part::Text {
                            text: "Build setup failed.".into(),
                        }],
                    },
                );
                let _ = task_manager.task_manager.transition_task(task_id, TaskState::Failed).await;
                let _ = sandbox::remove_worktree(&source_repo, &worktree).await;
                return;
            }
            tracing::info!(task_id = %task_id, "prompt written, launching claude");

            // Run Claude Code headless in the worktree
            let prompt_path = worktree.join(".relay-prompt.md");
            let agent_result = sandbox::run_sandboxed(
                "claude",
                &[
                    "--print",
                    "--dangerously-skip-permissions",
                    &format!("Read the file {} and implement the change request described in it. Follow all rules in CLAUDE.md strictly.", prompt_path.display()),
                ],
                &worktree,
                timeout,
            )
            .await;

            let claude_elapsed = start.elapsed();
            tracing::info!(
                task_id = %task_id,
                exit_code = agent_result.exit_code,
                timed_out = agent_result.timed_out,
                stdout_len = agent_result.stdout.len(),
                stderr_len = agent_result.stderr.len(),
                elapsed_secs = claude_elapsed.as_secs(),
                "claude session finished"
            );

            if agent_result.exit_code != 0 {
                tracing::warn!(
                    task_id = %task_id,
                    exit_code = agent_result.exit_code,
                    stderr = %agent_result.stderr.chars().take(500).collect::<String>(),
                    "claude exited with non-zero code"
                );
            }

            // Run cargo test
            tracing::info!(task_id = %task_id, "running cargo test");
            let test_result = agent_bridge::run_cargo_test(&worktree, timeout).await;
            let (passed, failed) = agent_bridge::parse_test_results(&test_result.stdout);

            tracing::info!(
                task_id = %task_id,
                test_exit_code = test_result.exit_code,
                tests_passed = passed,
                tests_failed = failed,
                "cargo test finished"
            );

            if test_result.exit_code != 0 {
                tracing::warn!(
                    task_id = %task_id,
                    stderr = %test_result.stderr.chars().take(500).collect::<String>(),
                    "cargo test failed"
                );
            }

            // Build the filtered summary
            let raw_summary = if agent_result.timed_out {
                "Build timed out.".to_string()
            } else {
                agent_result.stdout.clone()
            };
            let filtered = response_filter::filter_response(&raw_summary);

            let success = test_result.exit_code == 0 && failed == 0 && !agent_result.timed_out;

            // Build the structured report
            let build_report = report::BuildReport {
                branch: branch_name.clone(),
                success,
                summary: filtered,
                tests: report::TestReport {
                    total: passed + failed,
                    passed,
                    failed,
                },
                artifact: None, // TODO: compile and sign artifact
            };

            // Add build report as agent response
            let _ = task_manager.task_manager.add_message(
                task_id,
                Message {
                    role: MessageRole::Agent,
                    parts: vec![Part::Data {
                        mime_type: "application/vnd.oip.build-report+json".into(),
                        data: serde_json::to_value(&build_report).unwrap(),
                    }],
                },
            );

            // Transition based on test results
            let final_state = if success {
                let _ = task_manager.task_manager.transition_task(task_id, TaskState::Completed).await;

                // Keep the branch for promotion — write metadata to promotions queue
                let metadata = serde_json::json!({
                    "task_id": task_id.to_string(),
                    "branch": branch_name,
                    "developer": developer,
                    "description": intent_desc,
                    "tests_passed": passed,
                    "tests_failed": failed,
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                });
                let promotions_path = std::path::Path::new("/opt/promotions/pending");
                if let Err(e) = tokio::fs::create_dir_all(promotions_path).await {
                    tracing::warn!(error = %e, "could not create promotions directory");
                }
                let meta_file = promotions_path.join(format!("{}.json", task_id));
                if let Err(e) = tokio::fs::write(&meta_file, metadata.to_string()).await {
                    tracing::warn!(error = %e, "could not write promotion metadata");
                } else {
                    tracing::info!(task_id = %task_id, branch = %branch_name, "branch preserved for promotion review");
                }

                "completed"
            } else {
                let _ = task_manager.task_manager.transition_task(task_id, TaskState::Failed).await;
                // Failed builds: clean up the worktree
                let _ = sandbox::remove_worktree(&source_repo, &worktree).await;
                "failed"
            };

            // Only clean up worktree on failure — successful builds keep the branch
            if success {
                // Remove the worktree directory but keep the branch ref
                // The branch persists in the bare repo for promotion review
                let _ = sandbox::remove_worktree(&source_repo, &worktree).await;
            }

            let total_elapsed = start.elapsed();
            tracing::info!(
                task_id = %task_id,
                final_state = final_state,
                tests_passed = passed,
                tests_failed = failed,
                total_secs = total_elapsed.as_secs(),
                "build pipeline complete"
            );
        });
    } else {
        tracing::warn!(task_id = %task_id, "no source_repo configured — task stays in submitted state");
    }

    // Return task + clarity report
    Json(JsonRpcResponse::success(
        req.id,
        serde_json::json!({
            "task": task,
            "clarity": clarity,
        }),
    ))
    .into_response()
}

/// message/stream — SSE stream of task progress
async fn handle_message_stream(
    state: Arc<AppState>,
    claims: RelayClaims,
    req: JsonRpcRequest,
) -> Response {
    let task_id: uuid::Uuid = match req.params.get("taskId").and_then(|v| v.as_str()) {
        Some(id) => match id.parse() {
            Ok(u) => u,
            Err(_) => {
                return Json(JsonRpcResponse::error(
                    req.id,
                    -32602,
                    "invalid taskId format",
                ))
                .into_response()
            }
        },
        None => {
            return Json(JsonRpcResponse::error(
                req.id,
                -32602,
                "missing taskId parameter",
            ))
            .into_response()
        }
    };

    // FIX #5: Only return tasks owned by the requesting developer
    let task = match state.task_manager.get_task_for_owner(task_id, &claims.sub).await {
        Some(t) => t,
        None => {
            return Json(JsonRpcResponse::error(
                req.id,
                ERR_TASK_NOT_FOUND,
                "task not found",
            ))
            .into_response()
        }
    };

    let event = axum::response::sse::Event::default()
        .json_data(serde_json::json!({
            "jsonrpc": "2.0",
            "result": task,
        }))
        .unwrap();

    let stream = tokio_stream::once(Ok::<_, std::convert::Infallible>(event));
    Sse::new(stream).into_response()
}

/// tasks/get — retrieve a task by ID
async fn handle_tasks_get(
    state: Arc<AppState>,
    claims: RelayClaims,
    req: JsonRpcRequest,
) -> Response {
    let task_id: uuid::Uuid = match req.params.get("taskId").and_then(|v| v.as_str()) {
        Some(id) => match id.parse() {
            Ok(u) => u,
            Err(_) => {
                return Json(JsonRpcResponse::error(
                    req.id,
                    -32602,
                    "invalid taskId format",
                ))
                .into_response()
            }
        },
        None => {
            return Json(JsonRpcResponse::error(
                req.id,
                -32602,
                "missing taskId parameter",
            ))
            .into_response()
        }
    };

    // FIX #5: Only return tasks owned by the requesting developer
    match state.task_manager.get_task_for_owner(task_id, &claims.sub).await {
        Some(task) => Json(JsonRpcResponse::success(
            req.id,
            serde_json::to_value(&task).unwrap(),
        ))
        .into_response(),
        None => Json(JsonRpcResponse::error(
            req.id,
            ERR_TASK_NOT_FOUND,
            "task not found",
        ))
        .into_response(),
    }
}

/// tasks/cancel — cancel a running task
async fn handle_tasks_cancel(
    state: Arc<AppState>,
    claims: RelayClaims,
    req: JsonRpcRequest,
) -> Response {
    let task_id: uuid::Uuid = match req.params.get("taskId").and_then(|v| v.as_str()) {
        Some(id) => match id.parse() {
            Ok(u) => u,
            Err(_) => {
                return Json(JsonRpcResponse::error(
                    req.id,
                    -32602,
                    "invalid taskId format",
                ))
                .into_response()
            }
        },
        None => {
            return Json(JsonRpcResponse::error(
                req.id,
                -32602,
                "missing taskId parameter",
            ))
            .into_response()
        }
    };

    // FIX #5: Only allow canceling tasks owned by the requesting developer
    match state
        .task_manager
        .transition_task_for_owner(task_id, &claims.sub, TaskState::Canceled)
        .await
    {
        Ok(task) => Json(JsonRpcResponse::success(
            req.id,
            serde_json::to_value(&task).unwrap(),
        ))
        .into_response(),
        Err(e) => {
            let code = if e.contains("not found") {
                ERR_TASK_NOT_FOUND
            } else {
                ERR_TASK_NOT_CANCELABLE
            };
            Json(JsonRpcResponse::error(req.id, code, e)).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use axum::body::Body;
    use axum::http::{self, Request, StatusCode};
    use tower::ServiceExt;

    fn test_state() -> Arc<AppState> {
        let config = ServerConfig::test();
        Arc::new(AppState {
            task_manager: crate::task_manager::TaskManager::new(),
            rate_limiter: crate::rate_limit::RateLimiter::new(config.rate_limits.clone()),
            config,
        })
    }

    fn auth_header(secret: &str) -> String {
        let token = RelayClaims::sign("dev_test", "community", secret);
        format!("Bearer bsk_{}", token)
    }

    fn auth_header_for(sub: &str, secret: &str) -> String {
        let token = RelayClaims::sign(sub, "community", secret);
        format!("Bearer bsk_{}", token)
    }

    #[tokio::test]
    async fn health_check() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn agent_card_endpoint() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/agent.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let card: AgentCard = serde_json::from_slice(&body).unwrap();
        assert_eq!(card.name, "oz-relay-arcflow");
        assert!(card.capabilities.streaming);
    }

    #[tokio::test]
    async fn a2a_requires_auth() {
        let app = build_router(test_state());
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": {}
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/a2a")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn message_send_creates_task() {
        let state = test_state();
        let app = build_router(state.clone());

        let intent = oz_relay_common::intent::Intent {
            description: "Add trim() function".into(),
            motivation: "Need to trim whitespace from strings".into(),
            category: oz_relay_common::intent::IntentCategory::Feature,
            test_cases: vec![oz_relay_common::intent::TestCase {
                query: "RETURN trim('  hello  ')".into(),
                expected_behavior: "Returns 'hello'".into(),
                input_data: None,
            }],
            context: oz_relay_common::intent::IntentContext {
                arcflow_version: "1.7.0".into(),
                ..Default::default()
            },
        };
        let message = intent.into_message();

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": message,
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/a2a")
                    .header("content-type", "application/json")
                    .header(
                        "authorization",
                        auth_header(&ServerConfig::test().jwt_secret),
                    )
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let rpc: JsonRpcResponse = serde_json::from_slice(&body).unwrap();
        assert!(rpc.error.is_none());
        let result = rpc.result.unwrap();
        // Response now includes both task and clarity report
        let task: Task = serde_json::from_value(result["task"].clone()).unwrap();
        assert_eq!(task.state, TaskState::Submitted);
        assert_eq!(task.owner, "dev_test");
        assert_eq!(task.messages.len(), 1);
        // Verify clarity report is present
        let clarity = &result["clarity"];
        assert!(clarity["branch"].as_str().unwrap().starts_with("relay/dev_test-"));
        assert!(!clarity["understoodAs"].as_str().unwrap().is_empty());
        assert_eq!(clarity["testCriteria"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn invalid_intent_rejected() {
        let state = test_state();
        let app = build_router(state);

        let message = Message {
            role: MessageRole::User,
            parts: vec![Part::Data {
                mime_type: oz_relay_common::intent::INTENT_MIME_TYPE.into(),
                data: serde_json::json!({
                    "description": "",
                    "motivation": "test",
                    "category": "feature",
                    "test_cases": [{"query": "x", "expected_behavior": "y"}],
                    "context": {"arcflow_version": "1.7.0"}
                }),
            }],
        };

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "message/send",
            "params": message,
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/a2a")
                    .header("content-type", "application/json")
                    .header(
                        "authorization",
                        auth_header(&ServerConfig::test().jwt_secret),
                    )
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let rpc: JsonRpcResponse = serde_json::from_slice(&body).unwrap();
        assert!(rpc.error.is_some());
        assert_eq!(rpc.error.unwrap().code, ERR_INVALID_INTENT);
    }

    #[tokio::test]
    async fn unknown_method_rejected() {
        let state = test_state();
        let app = build_router(state);

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "nonexistent/method",
            "params": {}
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/a2a")
                    .header("content-type", "application/json")
                    .header(
                        "authorization",
                        auth_header(&ServerConfig::test().jwt_secret),
                    )
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let rpc: JsonRpcResponse = serde_json::from_slice(&body).unwrap();
        assert!(rpc.error.is_some());
        assert_eq!(rpc.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn tasks_get_returns_task() {
        let state = test_state();
        let msg = Message {
            role: MessageRole::User,
            parts: vec![Part::Text {
                text: "test".into(),
            }],
        };
        let task = state.task_manager.create_task("dev_test", msg).await;

        let app = build_router(state.clone());
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tasks/get",
            "params": {"taskId": task.id.to_string()}
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/a2a")
                    .header("content-type", "application/json")
                    .header(
                        "authorization",
                        auth_header(&ServerConfig::test().jwt_secret),
                    )
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let rpc: JsonRpcResponse = serde_json::from_slice(&body).unwrap();
        assert!(rpc.error.is_none());
        let fetched: Task = serde_json::from_value(rpc.result.unwrap()).unwrap();
        assert_eq!(fetched.id, task.id);
    }

    #[tokio::test]
    async fn tasks_get_not_found() {
        let state = test_state();
        let app = build_router(state);

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tasks/get",
            "params": {"taskId": uuid::Uuid::new_v4().to_string()}
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/a2a")
                    .header("content-type", "application/json")
                    .header(
                        "authorization",
                        auth_header(&ServerConfig::test().jwt_secret),
                    )
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let rpc: JsonRpcResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(rpc.error.unwrap().code, ERR_TASK_NOT_FOUND);
    }

    // FIX #5: Tenant isolation test — developer B cannot see developer A's tasks
    #[tokio::test]
    async fn tenant_isolation_enforced() {
        let state = test_state();
        let secret = &ServerConfig::test().jwt_secret;

        // Developer A creates a task
        let msg = Message {
            role: MessageRole::User,
            parts: vec![Part::Text {
                text: "test".into(),
            }],
        };
        let task = state.task_manager.create_task("dev_alice", msg).await;

        // Developer B tries to access it
        let app = build_router(state.clone());
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tasks/get",
            "params": {"taskId": task.id.to_string()}
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/a2a")
                    .header("content-type", "application/json")
                    .header("authorization", auth_header_for("dev_bob", secret))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let rpc: JsonRpcResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(rpc.error.unwrap().code, ERR_TASK_NOT_FOUND);
    }

    #[tokio::test]
    async fn rate_limiting_enforced() {
        let state = test_state();

        for i in 0..3 {
            let app = build_router(state.clone());
            let intent = oz_relay_common::intent::Intent {
                description: format!("intent {}", i),
                motivation: "test".into(),
                category: oz_relay_common::intent::IntentCategory::Feature,
                test_cases: vec![oz_relay_common::intent::TestCase {
                    query: "MATCH (n) RETURN n".into(),
                    expected_behavior: "returns nodes".into(),
                    input_data: None,
                }],
                context: oz_relay_common::intent::IntentContext {
                    arcflow_version: "1.7.0".into(),
                    ..Default::default()
                },
            };

            let body = serde_json::json!({
                "jsonrpc": "2.0",
                "id": i,
                "method": "message/send",
                "params": intent.into_message(),
            });

            let resp = app
                .oneshot(
                    Request::builder()
                        .method(http::Method::POST)
                        .uri("/a2a")
                        .header("content-type", "application/json")
                        .header(
                            "authorization",
                            auth_header(&ServerConfig::test().jwt_secret),
                        )
                        .body(Body::from(serde_json::to_string(&body).unwrap()))
                        .unwrap(),
                )
                .await
                .unwrap();

            let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            let rpc: JsonRpcResponse = serde_json::from_slice(&body_bytes).unwrap();
            assert!(rpc.error.is_none(), "request {} should succeed", i);
        }

        // 4th request should be rate limited
        let app = build_router(state.clone());
        let intent = oz_relay_common::intent::Intent {
            description: "one more".into(),
            motivation: "test".into(),
            category: oz_relay_common::intent::IntentCategory::Feature,
            test_cases: vec![oz_relay_common::intent::TestCase {
                query: "MATCH (n) RETURN n".into(),
                expected_behavior: "returns nodes".into(),
                input_data: None,
            }],
            context: oz_relay_common::intent::IntentContext {
                arcflow_version: "1.7.0".into(),
                ..Default::default()
            },
        };

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "message/send",
            "params": intent.into_message(),
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/a2a")
                    .header("content-type", "application/json")
                    .header(
                        "authorization",
                        auth_header(&ServerConfig::test().jwt_secret),
                    )
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let rpc: JsonRpcResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(rpc.error.unwrap().code, ERR_RATE_LIMITED);
    }

    // Entitlement gate tests

    #[tokio::test]
    async fn pending_key_cannot_submit() {
        let state = test_state();
        let app = build_router(state);
        let secret = &ServerConfig::test().jwt_secret;

        let intent = oz_relay_common::intent::Intent {
            description: "Add something".into(),
            motivation: "Need it".into(),
            category: oz_relay_common::intent::IntentCategory::Feature,
            test_cases: vec![oz_relay_common::intent::TestCase {
                query: "MATCH (n) RETURN n".into(),
                expected_behavior: "returns nodes".into(),
                input_data: None,
            }],
            context: oz_relay_common::intent::IntentContext {
                arcflow_version: "1.7.0".into(),
                ..Default::default()
            },
        };

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": intent.into_message(),
        });

        // Pending key should be rejected
        let pending_token = RelayClaims::sign_with_status("dev_pending", "community", "pending", secret);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/a2a")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer bsk_{}", pending_token))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK); // JSON-RPC returns 200 with error payload
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let rpc: JsonRpcResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(rpc.error.unwrap().code, ERR_KEY_NOT_ACTIVE);
    }

    #[tokio::test]
    async fn pending_key_can_read_tasks() {
        let state = test_state();
        let secret = &ServerConfig::test().jwt_secret;

        // Create a task as the pending developer (via internal API)
        let msg = Message {
            role: MessageRole::User,
            parts: vec![Part::Text { text: "test".into() }],
        };
        let task = state.task_manager.create_task("dev_pending", msg).await;

        let app = build_router(state.clone());
        let pending_token = RelayClaims::sign_with_status("dev_pending", "community", "pending", secret);

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/get",
            "params": {"taskId": task.id.to_string()}
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/a2a")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer bsk_{}", pending_token))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let rpc: JsonRpcResponse = serde_json::from_slice(&body).unwrap();
        assert!(rpc.error.is_none(), "pending key should be able to read tasks");
    }

    #[tokio::test]
    async fn suspended_key_rejected_at_auth() {
        let state = test_state();
        let app = build_router(state);
        let secret = &ServerConfig::test().jwt_secret;

        let suspended_token = RelayClaims::sign_with_status("dev_bad", "community", "suspended", secret);

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/get",
            "params": {"taskId": "00000000-0000-0000-0000-000000000000"}
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/a2a")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer bsk_{}", suspended_token))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Suspended keys get 403 at the middleware level
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
