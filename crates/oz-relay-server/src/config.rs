// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Server configuration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub relay_url: String,
    pub jwt_secret: String,
    pub rate_limits: RateLimitConfig,
    pub nsjail_config: Option<String>,
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
        }
    }
}
