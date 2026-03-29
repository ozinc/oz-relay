// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Server configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub relay_url: String,
    pub jwt_secret: String,
    pub rate_limits: RateLimitConfig,
    pub nsjail_config: Option<String>,
    pub source_repo: Option<PathBuf>,
    pub sandbox_timeout_secs: u64,
    /// Root directory for tasks, ledger, promotions, bugs.
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub community_per_day: u32,
    pub professional_per_day: u32,
    pub enterprise_per_day: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            community_per_day: 3,
            professional_per_day: 10,
            enterprise_per_day: 1000,
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            bind_addr: std::env::var("RELAY_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3400".into()),
            relay_url: std::env::var("RELAY_URL")
                .unwrap_or_else(|_| "http://localhost:3400".into()),
            jwt_secret: std::env::var("RELAY_JWT_SECRET")
                .unwrap_or_else(|_| "dev-secret-change-in-production".into()),
            rate_limits: RateLimitConfig::default(),
            nsjail_config: std::env::var("RELAY_NSJAIL_CONFIG").ok(),
            source_repo: std::env::var("RELAY_SOURCE_REPO").ok().map(PathBuf::from),
            sandbox_timeout_secs: std::env::var("RELAY_SANDBOX_TIMEOUT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1800),
            data_dir: std::env::var("RELAY_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/opt/oz-relay")),
        }
    }

    /// Config for testing with a known secret.
    pub fn test() -> Self {
        Self {
            bind_addr: "127.0.0.1:0".into(),
            relay_url: "http://localhost:3400".into(),
            jwt_secret: "test-secret-key-for-unit-tests".into(),
            rate_limits: RateLimitConfig::default(),
            nsjail_config: None,
            source_repo: None,
            sandbox_timeout_secs: 1800,
            data_dir: std::env::temp_dir().join("oz-relay-test"),
        }
    }
}
