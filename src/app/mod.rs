mod accounts;
pub mod auth;
mod health;
mod transactions;
mod users;

use crate::app::auth::{JwtKeys, auth_middleware};
use crate::core::errors::{AppResult, Error};
use crate::core::types::IdempotencyKey;
use axum::Router;
use axum::extract::{rejection::JsonRejection, FromRequest, FromRequestParts, MatchedPath, Request};
use axum::http::request::Parts;
use axum::routing::{get, post};
use sqlx::PgPool;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use axum::middleware::Next;
use axum::response::Response;
use metrics::{counter, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tower_http::cors::CorsLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

#[derive(Clone)]
pub struct AppState {
  pub pool: PgPool,
  pub jwt: Arc<JwtKeys>,
}

impl AppState {
  pub fn new(
    pool: PgPool,
    jwt: Arc<JwtKeys>,
  ) -> Self {
    Self { pool, jwt }
  }
}

static METRICS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

pub fn setup_metrics() -> PrometheusHandle {
  METRICS_HANDLE
    .get_or_init(|| {
      PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus recorder")
    })
    .clone()
}

async fn index() -> axum::response::Html<&'static str> {
  axum::response::Html(include_str!("../../static/index.html"))
}

pub fn router(state: AppState, prometheus: PrometheusHandle) -> Router {
  let trace_layer = TraceLayer::new_for_http()
    .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
    .on_response(
      DefaultOnResponse::new()
        .level(Level::INFO)
        .latency_unit(tower_http::LatencyUnit::Millis),
    );

  let protected = Router::new()
    // Accounts
    .route(
      "/accounts",
      post(accounts::create_account).get(accounts::list_accounts),
    )
    .route("/accounts/{id}", get(accounts::get_account))
    .route("/accounts/{id}/balance", get(accounts::get_balance))
    // Transactions
    .route("/transactions", post(transactions::transfer))
    .route("/transactions/{id}/reversal", post(transactions::reverse))
    .route("/accounts/{id}/transactions", get(accounts::get_activity))
    .route_layer(axum::middleware::from_fn_with_state(
      state.clone(),
      auth_middleware,
    ));

  let public = Router::new()
    .route("/", get(index))
    .route("/health/live", get(health::liveness))
    .route("/health/ready", get(health::readiness))
    .route("/users/register", post(users::register))
    .route("/users/login", post(users::login));

  let metrics = Router::new()
    .route("/metrics", get(health::metrics))
    .with_state(prometheus);

  Router::new()
    .merge(public)
    .merge(protected)
    .merge(metrics)
    .layer(axum::middleware::from_fn(track_metrics))
    .layer(trace_layer)
    .layer(CorsLayer::permissive())
    .with_state(state)
}

pub async fn serve(state: AppState, port: u16, prometheus: PrometheusHandle) -> AppResult<()> {
  let addr = format!("0.0.0.0:{}", port);
  let listener = tokio::net::TcpListener::bind(&addr)
    .await
    .map_err(|e| Error::Internal(e.to_string()))?;

  tracing::info!(
    addr = %addr,
    max_connections = state.pool.options().get_max_connections(),
    "ledger service initialized and listening"
  );

  axum::serve(listener, router(state, prometheus))
    .await
    .map_err(|e| Error::Internal(e.to_string()))?;

  Ok(())
}

pub async fn track_metrics(req: Request, next: Next) -> Response {
  let start = Instant::now();

  // Extract path template (e.g., "/accounts/{id}") to avoid metric cardinality explosion
  let path = if let Some(matched_path) = req.extensions().get::<MatchedPath>() {
    matched_path.as_str().to_owned()
  } else {
    req.uri().path().to_owned()
  };

  let method = req.method().clone();
  let response = next.run(req).await;
  let latency = start.elapsed().as_secs_f64();
  let status = response.status().as_u16().to_string();

  let labels = [
    ("method", method.to_string()),
    ("path", path),
    ("status", status),
  ];

  counter!("http_requests_total", &labels).increment(1);
  histogram!("http_request_duration_seconds", &labels).record(latency);

  response
}

// Extractor reading "idempotency-key" header
impl<S> FromRequestParts<S> for IdempotencyKey
where
  S: Send + Sync,
{
  type Rejection = Error;

  async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
    let raw = parts
      .headers
      .get("idempotency-key")
      .and_then(|val| val.to_str().ok())
      .ok_or_else(|| Error::BadRequest("idempotency-key not found in headers".into()))?;

    // TryFrom<String> will check character count and return BadRequest on error
    IdempotencyKey::try_from(raw.to_string())
  }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AppJson<T>(pub T);

impl<T, S> FromRequest<S> for AppJson<T>
where
  axum::Json<T>: FromRequest<S, Rejection = JsonRejection>,
  S: Send + Sync,
{
  type Rejection = Error;

  async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
    match axum::Json::<T>::from_request(req, state).await {
      Ok(axum::Json(val)) => Ok(AppJson(val)),
      Err(rejection) => Err(Error::BadRequest(rejection.to_string())),
    }
  }
}
