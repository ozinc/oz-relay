// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! OZ Relay Server — A2A-compliant build relay for agent-mediated contributions.

mod agent_bridge;
#[allow(dead_code)]
mod artifact_signer;
mod auth;
mod config;
mod rate_limit;
mod response_filter;
mod routes;
mod sandbox;
mod task_manager;

use std::sync::Arc;

use axum::Router;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::config::ServerConfig;
use crate::rate_limit::RateLimiter;
use crate::task_manager::TaskManager;

/// Shared application state.
pub struct AppState {
    pub task_manager: TaskManager,
    pub rate_limiter: RateLimiter,
    pub config: ServerConfig,
}

#[tokio::main]
async fn main() {
    // Initialize structured logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(true)
        .with_thread_ids(false)
        .init();

    let config = ServerConfig::from_env();
    let addr = config.bind_addr.clone();

    tracing::info!(
        bind = %addr,
        source_repo = ?config.source_repo,
        "oz-relay-server starting"
    );

    let state = Arc::new(AppState {
        task_manager: TaskManager::new(),
        rate_limiter: RateLimiter::new(config.rate_limits.clone()),
        config,
    });

    let app = routes::build_router(state);

    let listener = TcpListener::bind(&addr).await.unwrap();
    tracing::info!("oz-relay-server listening on {}", addr);
    axum::serve(listener, app).await.unwrap();
}

/// Build a router with the given state (used by tests too).
pub fn build_app(state: Arc<AppState>) -> Router {
    routes::build_router(state)
}
