// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Per-developer rate limiting by tier.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use crate::config::RateLimitConfig;

pub struct RateLimiter {
    config: RateLimitConfig,
    /// Map of developer_id → (count, window_start)
    counters: Mutex<HashMap<String, (u32, DateTime<Utc>)>>,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            counters: Mutex::new(HashMap::new()),
        }
    }

    /// Check if a request is allowed. Returns Ok(remaining) or Err(retry_after_secs).
    pub fn check(&self, developer_id: &str, tier: &str) -> Result<u32, u64> {
        let limit = match tier {
            "community" => self.config.community_per_day,
            "professional" => self.config.professional_per_day,
            "enterprise" => self.config.enterprise_per_day,
            _ => self.config.community_per_day,
        };

        let now = Utc::now();
        let mut counters = self.counters.lock().unwrap();
        let entry = counters
            .entry(developer_id.to_string())
            .or_insert((0, now));

        // Reset window if 24h has passed
        let window_duration = chrono::Duration::hours(24);
        if now - entry.1 > window_duration {
            *entry = (0, now);
        }

        if entry.0 >= limit {
            let reset_at = entry.1 + window_duration;
            let retry_after = (reset_at - now).num_seconds().max(0) as u64;
            return Err(retry_after);
        }

        entry.0 += 1;
        Ok(limit - entry.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RateLimitConfig;

    #[test]
    fn community_rate_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            community_per_day: 3,
            professional_per_day: 10,
            enterprise_per_day: 1000,
        });

        assert!(limiter.check("dev_001", "community").is_ok());
        assert!(limiter.check("dev_001", "community").is_ok());
        assert!(limiter.check("dev_001", "community").is_ok());
        assert!(limiter.check("dev_001", "community").is_err());
        assert!(limiter.check("dev_002", "community").is_ok());
    }

    #[test]
    fn professional_higher_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            community_per_day: 3,
            professional_per_day: 10,
            enterprise_per_day: 1000,
        });

        for _ in 0..10 {
            assert!(limiter.check("dev_pro", "professional").is_ok());
        }
        assert!(limiter.check("dev_pro", "professional").is_err());
    }

    #[test]
    fn remaining_count_decrements() {
        let limiter = RateLimiter::new(RateLimitConfig::default());
        assert_eq!(limiter.check("dev_x", "community").unwrap(), 2);
        assert_eq!(limiter.check("dev_x", "community").unwrap(), 1);
        assert_eq!(limiter.check("dev_x", "community").unwrap(), 0);
    }
}
