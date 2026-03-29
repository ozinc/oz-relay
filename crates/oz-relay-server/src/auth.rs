// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! JWT authentication for bsk_ API keys.

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;

/// JWT claims embedded in a bsk_ token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayClaims {
    /// Developer/organization ID.
    pub sub: String,
    /// Tier: "community", "professional", "enterprise".
    pub tier: String,
    /// Expiration (Unix timestamp).
    pub exp: u64,
}

impl RelayClaims {
    /// Create a signed JWT token. Used by tests and the CLI.
    #[allow(dead_code)]
    pub fn sign(sub: &str, tier: &str, secret: &str) -> String {
        let claims = Self {
            sub: sub.into(),
            tier: tier.into(),
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
            req.extensions_mut().insert(token_data.claims);
            Ok(next.run(req).await)
        }
        Err(_) => Err(StatusCode::UNAUTHORIZED),
    }
}
