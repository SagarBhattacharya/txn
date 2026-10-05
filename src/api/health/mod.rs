use axum::Router;
use axum::routing::get;
use crate::api::AppState;

mod routes;

pub fn health_router() -> Router<AppState> {
	Router::new()
		.route("/live", get(routes::liveness_probe))
		.route("/ready", get(routes::readiness_probe))
}