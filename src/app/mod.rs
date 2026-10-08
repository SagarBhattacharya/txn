mod accounts;
mod health;
mod metrics;
mod transactions;
mod users;

use crate::core::auth::{JwtKeys, auth_middleware};
use crate::core::errors::{AppResult, Error};
use crate::core::types::IdempotencyKey;
use axum::extract::{FromRequest, FromRequestParts, Request, rejection::JsonRejection};
use axum::http::request::Parts;
use axum::response::Html;
use axum::routing::get;
use axum::{Router, middleware};
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;
use std::sync::Arc;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

#[derive(Clone)]
pub struct AppState {
  pub pool: PgPool,
  pub jwt: Arc<JwtKeys>,
  pub prometheus: PrometheusHandle,
}

impl AppState {
  pub fn new(pool: PgPool, jwt: Arc<JwtKeys>) -> Self {
    Self {
      pool,
      jwt,
      prometheus: metrics::setup(),
    }
  }
}

pub struct App {
  state: AppState,
  router: Router,
}

impl App {
  pub fn from_state(state: AppState) -> AppResult<Self> {
    let trace_layer = TraceLayer::new_for_http()
      .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
      .on_response(
        DefaultOnResponse::new()
          .level(Level::INFO)
          .latency_unit(tower_http::LatencyUnit::Millis),
      );

    let protected = Router::new()
      .nest("/accounts", accounts::router())
      .nest("/transactions", transactions::router())
      .route_layer(middleware::from_fn_with_state(
        state.clone(),
        auth_middleware,
      ));

    let public = Router::new()
      .route("/", get(index))
      .route("/metrics", get(metrics::handler))
      .nest("/health", health::router())
      .nest("/users", users::router());

    let router = Router::new()
      .merge(public)
      .merge(protected)
      .layer(middleware::from_fn(metrics::track))
      .layer(trace_layer)
      .with_state(state.clone());

    Ok(Self { state, router })
  }

  pub fn router(&self) -> Router {
    self.router.clone()
  }

  pub async fn run(self, port: u16) -> AppResult<()> {
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
      .await
      .map_err(|e| Error::Internal(e.to_string()))?;

    tracing::info!(
      addr = %addr,
      max_connections = self.state.pool.options().get_max_connections(),
      "ledger service initialized and listening"
    );

    axum::serve(listener, self.router)
      .with_graceful_shutdown(shutdown_signal())
      .await
      .map_err(|e| Error::Internal(e.to_string()))?;

    Ok(())
  }
}

async fn index() -> Html<&'static str> {
  Html(include_str!("../../static/index.html"))
}

async fn shutdown_signal() {
  let ctrl_c = async {
    tokio::signal::ctrl_c()
      .await
      .expect("failed to install Ctrl+C handler");
  };

  #[cfg(unix)]
  let terminate = async {
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
      .expect("failed to install SIGTERM handler")
      .recv()
      .await;
  };

  #[cfg(not(unix))]
  let terminate = std::future::pending::<()>();

  tokio::select! {
    _ = ctrl_c => {},
    _ = terminate => {},
  }

  tracing::info!("Shutdown signal received, draining connections...");
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
