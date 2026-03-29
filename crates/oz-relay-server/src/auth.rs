// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! JWT authentication and entitlement gating for bsk_ API keys.
//!
//! Keys have three statuses:
//! - `active`: authorized to submit intents (burns server-side tokens)
//! - `pending`: key issued but developer hasn't been approved yet (read-only)
//! - `suspended`: key revoked by OZ (rejected)
//!
//! The entitlement check happens in the auth middleware, before any
//! compute-expensive operations. Pending/suspended keys can still call
//! tasks/get and tasks/cancel but cannot submit new intents.

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;

/// Valid key statuses.
pub const STATUS_ACTIVE: &str = "active";
pub const STATUS_PENDING: &str = "pending";
pub const STATUS_SUSPENDED: &str = "suspended";

/// JWT claims embedded in a bsk_ token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayClaims {
    /// Developer/organization ID.
    pub sub: String,
    /// Tier: "community", "professional", "enterprise".
    pub tier: String,
    /// Key status: "active", "pending", "suspended".
    #[serde(default = "default_status")]
    pub status: String,
    /// Expiration (Unix timestamp).
    pub exp: u64,
}

fn default_status() -> String {
    // Keys without a status field (pre-entitlement) default to active
    // for backwards compatibility with already-issued tokens.
    STATUS_ACTIVE.into()
}

impl RelayClaims {
    /// Returns true if this key is authorized to submit intents.
    pub fn is_active(&self) -> bool {
        self.status == STATUS_ACTIVE
    }

    /// Create a signed JWT token with active status. Used by tests.
    #[allow(dead_code)]
    pub fn sign(sub: &str, tier: &str, secret: &str) -> String {
        Self::sign_with_status(sub, tier, STATUS_ACTIVE, secret)
    }

    /// Create a signed JWT token with explicit status.
    #[allow(dead_code)]
    pub fn sign_with_status(sub: &str, tier: &str, status: &str, secret: &str) -> String {
        let claims = Self {
            sub: sub.into(),
            tier: tier.into(),
            status: status.into(),
            exp: (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as u64,
        };
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }
}

/// Axum middleware that validates Bearer tokens.
///
/// Rejects suspended keys outright (403). Allows pending and active keys
/// through — the entitlement check for pending keys happens at the
/// route level (message/send rejects pending, tasks/get allows it).
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => return Err(StatusCode::UNAUTHORIZED),
    };

    // Strip bsk_ prefix if present
    let jwt = if let Some(stripped) = token.strip_prefix("bsk_") {
        stripped
    } else {
        token
    };

    let key = DecodingKey::from_secret(state.config.jwt_secret.as_bytes());
    let validation = Validation::new(Algorithm::HS256);

    match decode::<RelayClaims>(jwt, &key, &validation) {
        Ok(token_data) => {
            // Suspended keys are rejected immediately
            if token_data.claims.status == STATUS_SUSPENDED {
                return Err(StatusCode::FORBIDDEN);
            }
            req.extensions_mut().insert(token_data.claims);
            Ok(next.run(req).await)
        }
        Err(_) => Err(StatusCode::UNAUTHORIZED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_status_is_active() {
        // Tokens without status field should deserialize as active
        let json = r#"{"sub":"dev_test","tier":"community","exp":9999999999}"#;
        let claims: RelayClaims = serde_json::from_str(json).unwrap();
        assert_eq!(claims.status, STATUS_ACTIVE);
        assert!(claims.is_active());
    }

    #[test]
    fn pending_key_not_active() {
        let json = r#"{"sub":"dev_test","tier":"community","status":"pending","exp":9999999999}"#;
        let claims: RelayClaims = serde_json::from_str(json).unwrap();
        assert_eq!(claims.status, STATUS_PENDING);
        assert!(!claims.is_active());
    }

    #[test]
    fn explicit_active_is_active() {
        let json = r#"{"sub":"dev_test","tier":"community","status":"active","exp":9999999999}"#;
        let claims: RelayClaims = serde_json::from_str(json).unwrap();
        assert!(claims.is_active());
    }
}
