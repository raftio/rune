pub mod routes;
pub mod error;
pub mod middleware;

pub use error::GatewayError;
pub use middleware::RateLimiter;

use axum::{middleware::from_fn, Router};
use rune_env::PlatformEnv;
use rune_storage::RuneStore;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<RuneStore>,
    pub backend: Option<Arc<dyn rune_runtime::RuntimeBackend>>,
    pub env: Arc<PlatformEnv>,
}

pub fn router(
    store: Arc<RuneStore>,
    backend: Option<Arc<dyn rune_runtime::RuntimeBackend>>,
    prometheus_handle: Option<metrics_exporter_prometheus::PrometheusHandle>,
    env: Arc<PlatformEnv>,
) -> Router {
    use axum::routing::get;
    use axum::routing::post;

    let limiter = middleware::RateLimiter::new(env.rate_limit_rps, env.rate_limit_burst);
    let rate_limit_layer = from_fn(move |req, next| {
        let limiter = limiter.clone();
        async move { middleware::rate_limit(req, next, limiter).await }
    });

    let state = AppState {
        store,
        backend,
        env: env.clone(),
    };

    let auth_env = env.clone();
    let auth_layer = from_fn(move |req, next| {
        let env = auth_env.clone();
        middleware::auth(req, next, env)
    });

    let mut app = Router::new()
        .route("/v1/agents/:agent_name/invoke", post(routes::invoke::invoke))
        .route("/v1/agents/:agent_name/sessions", post(routes::sessions::create_session))
        .route("/v1/sessions/:session_id", get(routes::sessions::get_session))
        .route("/v1/replicas/:replica_id/health", get(routes::health::replica_health))
        .route("/.well-known/agent.json", get(routes::a2a::agent_card_global))
        .route("/a2a/:agent_name/agent-card", get(routes::a2a::agent_card))
        .route("/a2a/:agent_name", post(routes::a2a::jsonrpc_handler))
        .route("/canvas/:id", get(routes::canvas::serve_canvas))
        .route("/mcp", post(routes::mcp::handle_mcp));

    if let Some(handle) = prometheus_handle {
        app = app.route("/metrics", get(move || {
            let h = handle.clone();
            async move {
                axum::response::Response::builder()
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(axum::body::Body::from(h.render()))
                    .unwrap()
            }
        }));
    }

    app.layer(rate_limit_layer)
        .layer(auth_layer)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
