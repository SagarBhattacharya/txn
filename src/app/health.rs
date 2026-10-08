use crate::app::{health, AppState};
use crate::db;
use axum::{extract::State, http::StatusCode, Router};
use axum::routing::get;
use metrics_exporter_prometheus::PrometheusHandle;

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