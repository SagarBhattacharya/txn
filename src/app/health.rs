use crate::app::AppState;
use crate::db;
use axum::{extract::State, http::StatusCode};
use metrics_exporter_prometheus::PrometheusHandle;

pub async fn liveness() -> StatusCode {
  StatusCode::OK
}

pub async fn readiness(State(state): State<AppState>) -> StatusCode {
  match db::ping(&state.pool).await {
    Ok(_) => StatusCode::OK,
    Err(_) => StatusCode::SERVICE_UNAVAILABLE,
  }
}

pub async fn metrics(State(metric): State<PrometheusHandle>) -> String {
  metric.render()
}