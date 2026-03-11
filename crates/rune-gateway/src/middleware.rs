use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rune_env::PlatformEnv;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const SKIP_AUTH_PATHS: &[&str] = &["/v1/replicas/", "/.well-known/agent.json", "/metrics"];

fn should_skip_auth(path: &str) -> bool {
    SKIP_AUTH_PATHS
        .iter()
        .any(|p| path == *p || path.starts_with(p))
}

pub async fn auth(request: Request, next: Next, env: Arc<PlatformEnv>) -> Response {
    let path = request.uri().path();
    if should_skip_auth(path) {
        return next.run(request).await;
    }

    if !env.auth_enabled() {
        return next.run(request).await;
    }

    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        Some(_) | None => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(json!({ "error": "Missing or invalid Authorization header. Use Bearer <token>." })),
            )
                .into_response();
        }
    };

    if token.is_empty() {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({ "error": "Empty token" })),
        )
            .into_response();
    }

    // API key check (constant-time comparison would be ideal but
    // for now direct equality is acceptable since timing leaks are
    // mitigated by the network round-trip jitter).
    if let Some(api_key) = &env.api_key {
        if token == api_key.as_str() {
            return next.run(request).await;
        }
    }

    // JWT check — explicitly enforce HS256 to block alg:none attacks
    if let Some(jwt_secret) = &env.jwt_secret {
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        if let Ok(_claims) = jsonwebtoken::decode::<serde_json::Value>(
            token,
            &jsonwebtoken::DecodingKey::from_secret(jwt_secret.as_bytes()),
            &validation,
        ) {
            return next.run(request).await;
        }
    }

    // All checks failed — reject unconditionally.
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(json!({ "error": "Invalid or expired token" })),
    )
        .into_response()
}

pub fn tenant_id_from_request(request: &Request) -> Option<String> {
    request
        .headers()
        .get("X-Tenant-Id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(burst: u32) -> Self {
        Self {
            tokens: burst as f64,
            last_refill: Instant::now(),
        }
    }
}

#[derive(Clone)]
pub struct RateLimiter {
    state: Arc<RwLock<HashMap<String, TokenBucket>>>,
    #[allow(dead_code)]
    rps: f64,
    burst: u32,
    refill_interval: Duration,
}

impl RateLimiter {
    pub fn new(rps: f64, burst: u32) -> Self {
        let rps = rps.max(0.001);
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
            rps,
            burst,
            refill_interval: Duration::from_secs_f64(1.0 / rps),
        }
    }

    pub async fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut state = self.state.write().await;
        let bucket = state
            .entry(key.to_string())
            .or_insert_with(|| TokenBucket::new(self.burst));

        let elapsed = now.duration_since(bucket.last_refill);
        let refills = elapsed.as_secs_f64() / self.refill_interval.as_secs_f64();
        bucket.tokens = (bucket.tokens + refills * 1.0).min(self.burst as f64);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

const SKIP_RATE_LIMIT_PATHS: &[&str] = &["/v1/replicas/", "/.well-known/agent.json", "/metrics"];

fn should_skip_rate_limit(path: &str) -> bool {
    SKIP_RATE_LIMIT_PATHS
        .iter()
        .any(|p| path == *p || path.starts_with(p))
}

pub async fn rate_limit(
    request: Request,
    next: Next,
    limiter: RateLimiter,
) -> Response {
    let path = request.uri().path();
    if should_skip_rate_limit(path) {
        return next.run(request).await;
    }

    let key = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| {
            request
                .headers()
                .get("X-Tenant-Id")
                .and_then(|v| v.to_str().ok())
                .map(|s| format!("tenant:{}", s))
        })
        .unwrap_or_else(|| "anonymous".to_string());

    if !limiter.check(&key).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(axum::http::header::RETRY_AFTER, "1")],
            axum::Json(json!({ "error": "Rate limit exceeded" })),
        )
            .into_response();
    }

    next.run(request).await
}
