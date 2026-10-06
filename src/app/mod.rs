mod accounts;
pub mod auth;
mod health;
mod transactions;
mod users;

use crate::app::auth::{JwtKeys, auth_middleware};
use crate::core::errors::{AppResult, Error};
use crate::core::types::IdempotencyKey;
use axum::Router;
use axum::extract::{FromRequest, FromRequestParts, Request, rejection::JsonRejection};
use axum::http::request::Parts;
use axum::routing::{get, post};
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
  pub pool: PgPool,
  pub jwt: Arc<JwtKeys>,
}

impl AppState {
  pub fn new(pool: PgPool, jwt: Arc<JwtKeys>) -> Self {
    Self { pool, jwt }
  }
}

pub fn router(state: AppState) -> Router {
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
    .route_layer(axum::middleware::from_fn_with_state(
      state.clone(),
      auth_middleware,
    ));

  let public = Router::new()
    .route("/health/live", get(health::liveness))
    .route("/health/ready", get(health::readiness))
    .route("/users/register", post(users::register))
    .route("/users/login", post(users::login));

  Router::new()
    .merge(public)
    .merge(protected)
    .with_state(state)
}

pub async fn serve(state: AppState, port: u16) -> AppResult<()> {
  let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
  let listener = tokio::net::TcpListener::bind(addr)
    .await
    .map_err(|e| Error::Internal(e.to_string()))?;

  axum::serve(listener, router(state))
    .await
    .map_err(|e| Error::Internal(e.to_string()))?;

  Ok(())
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
