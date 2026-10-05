use axum::{
	extract::State,
	http::StatusCode,
	response::{IntoResponse, Response},
	Json,
};
use serde_json::json;

use crate::api::AppState;

pub async fn liveness_probe() -> impl IntoResponse {
	(StatusCode::OK, Json(json!({ "status": "alive" })))
}

pub async fn readiness_probe(State(state): State<AppState>) -> Response {
	match sqlx::query("select 1").execute(state.repo.pool()).await {
		Ok(_) => (StatusCode::OK, Json(json!({ "status": "ready" }))).into_response(),
		Err(e) => (
			StatusCode::SERVICE_UNAVAILABLE,
			Json(json!({
        "status": "unhealthy",
        "reason": format!("Database unreachable: {e}")
      })),
		)
			.into_response(),
	}
}