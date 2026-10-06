use crate::app::AppState;
use crate::db::ping;
use axum::{extract::State, http::StatusCode};

pub async fn liveness() -> StatusCode {
  StatusCode::OK
}

pub async fn readiness(State(state): State<AppState>) -> StatusCode {
  match ping(&state.pool).await {
    Ok(_) => StatusCode::OK,
    Err(_) => StatusCode::SERVICE_UNAVAILABLE,
  }
}
