pub mod registry;
pub mod admission;
mod routes;
pub mod error;

pub use error::ControlPlaneError;

use axum::Router;
use rune_storage::RuneStore;
use std::sync::Arc;

pub fn router(store: Arc<RuneStore>) -> Router {
    use axum::routing::{delete, get, post};

    Router::new()
        .route("/v1/agent-versions", post(routes::registry::register_version))
        .route("/v1/agent-versions", get(routes::registry::list_versions))
        .route("/v1/deployments", post(routes::deployments::create_deployment))
        .route("/v1/deployments", get(routes::deployments::list_deployments))
        .route("/v1/deployments/:id/scale", post(routes::deployments::scale_deployment))
        .route("/v1/deployments/:id", delete(routes::deployments::delete_deployment))
        .with_state(store)
}
