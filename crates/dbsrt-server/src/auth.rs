//! Optional shared-key authentication for every route except the health probes.

use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// Liveness probes stay public: the PHP proxy and the cron watchdog poll them without a key.
const PUBLIC_PATHS: [&str; 2] = ["/health", "/v1/health"];

/// Extracts the presented key from `Authorization: Bearer <key>` or `X-API-Key: <key>`.
fn presented_key(headers: &HeaderMap) -> Option<&str> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    bearer
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()))
        .map(str::trim)
}

/// Constant-time byte comparison, so response timing does not leak a key prefix.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Matches `candidate` against the key pool: one key per line, blank and `#` lines ignored.
pub fn key_matches(pool: &str, candidate: &str) -> bool {
    !candidate.is_empty()
        && pool
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .fold(false, |hit, key| {
                ct_eq(key.as_bytes(), candidate.as_bytes()) | hit
            })
}

/// Axum middleware: open when no key file is configured, else 401 unless a pool key is presented.
///
/// The file is re-read per request so a key rotation needs no restart; an unreadable file
/// fails closed with 503.
pub async fn require_api_key(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(path) = state.api_key_file.as_deref() else {
        return next.run(req).await;
    };
    if PUBLIC_PATHS.contains(&req.uri().path()) {
        return next.run(req).await;
    }
    let pool = match tokio::fs::read_to_string(path).await {
        Ok(pool) => pool,
        Err(e) => {
            tracing::error!("API key file {:?} unreadable: {}", path, e);
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    match presented_key(req.headers()) {
        Some(key) if key_matches(&pool, key) => next.run(req).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_matches_pool_rules() {
        let pool = "# comment\n\n  alpha  \nbeta\n";
        assert!(key_matches(pool, "alpha"));
        assert!(key_matches(pool, "beta"));
        assert!(!key_matches(pool, "alph"));
        assert!(!key_matches(pool, "# comment"));
        assert!(!key_matches(pool, ""));
        assert!(!key_matches("", "alpha"));
    }

    #[test]
    fn test_presented_key_sources() {
        let mut h = HeaderMap::new();
        assert_eq!(presented_key(&h), None);
        h.insert("x-api-key", "k1".parse().unwrap());
        assert_eq!(presented_key(&h), Some("k1"));
        h.insert(header::AUTHORIZATION, "Bearer k2".parse().unwrap());
        assert_eq!(presented_key(&h), Some("k2"));
        h.insert(header::AUTHORIZATION, "Basic xyz".parse().unwrap());
        assert_eq!(presented_key(&h), Some("k1"));
    }
}
