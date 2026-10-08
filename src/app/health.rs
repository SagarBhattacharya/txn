use crate::app::AppState;
use crate::db;
use axum::routing::get;
use axum::{Router, extract::State, http::StatusCode};

pub fn router() -> Router<AppState> {
  Router::new()
    .route("/live", get(liveness))
    .route("/ready", get(readiness))
}

async fn liveness() -> StatusCode {
  StatusCode::OK
}

async fn readiness(State(state): State<AppState>) -> StatusCode {
  match db::ping(&state.pool).await {
    Ok(_) => StatusCode::OK,
    Err(_) => StatusCode::SERVICE_UNAVAILABLE,
  }
}
