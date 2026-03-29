// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Per-developer rate limiting by tier.
//! Counters are reconstructed from the ledger on startup (RLY-0020).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};

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

    /// Reconstruct rate limit counters from the ledger.
    /// Counts `task.created` events per owner in the last 24 hours.
    pub fn load_from_ledger(&self, ledger_path: &Path) {
        let content = match std::fs::read_to_string(ledger_path) {
            Ok(c) => c,
            Err(_) => return, // No ledger yet
        };

        let now = Utc::now();
        let window = Duration::hours(24);
        let cutoff = now - window;

        let mut counts: HashMap<String, u32> = HashMap::new();

        for line in content.lines() {
            let event: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Only count task.created events
            if event.get("event").and_then(|v| v.as_str()) != Some("task.created") {
                continue;
            }

            // Parse timestamp
            let ts = match event.get("ts").and_then(|v| v.as_str()) {
                Some(ts) => match ts.parse::<DateTime<Utc>>() {
                    Ok(dt) => dt,
                    Err(_) => continue,
                },
                None => continue,
            };

            // Only count events in the current 24h window
            if ts < cutoff {
                continue;
            }

            if let Some(owner) = event.get("owner").and_then(|v| v.as_str()) {
                *counts.entry(owner.to_string()).or_insert(0) += 1;
            }
        }

        if !counts.is_empty() {
            let mut counters = self.counters.lock().unwrap();
            for (owner, count) in &counts {
                counters.insert(owner.clone(), (*count, now));
            }
            tracing::info!(
                developers = counts.len(),
                "rate limits restored from ledger"
            );
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
        let window_duration = Duration::hours(24);
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

    #[test]
    fn load_from_ledger_restores_counts() {
        let dir = std::env::temp_dir().join(format!("rly-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ledger = dir.join("events.jsonl");

        // Write some fake ledger entries
        let now = Utc::now().to_rfc3339();
        let content = format!(
            "{}\n{}\n{}\n",
            serde_json::json!({"ts": now, "event": "task.created", "owner": "dev_alice"}),
            serde_json::json!({"ts": now, "event": "task.created", "owner": "dev_alice"}),
            serde_json::json!({"ts": now, "event": "task.created", "owner": "dev_bob"}),
        );
        std::fs::write(&ledger, content).unwrap();

        let limiter = RateLimiter::new(RateLimitConfig {
            community_per_day: 3,
            professional_per_day: 10,
            enterprise_per_day: 1000,
        });
        limiter.load_from_ledger(&ledger);

        // Alice used 2, should have 1 remaining
        assert_eq!(limiter.check("dev_alice", "community").unwrap(), 0);
        // Bob used 1, should have 2 remaining
        assert_eq!(limiter.check("dev_bob", "community").unwrap(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
