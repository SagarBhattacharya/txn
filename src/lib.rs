use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::Error;
use thiserror::Error;
use validator::{ValidationErrors};

pub mod api;
pub mod config;
pub mod db;
pub mod models;
pub mod auth;

pub type LedgerResult<T> = Result<T, LedgerError>;

#[derive(Debug, Error, PartialEq)]
pub enum LedgerError {
  #[error("expected at least 2 postings, found {0}")]
  InsufficientPostings(i32),

  #[error("a posting cannot have an amount of 0.0")]
  ZeroAmountPosting,

  #[error("transaction is unbalanced: {0}")]
  UnbalancedTransaction(Decimal),

  #[error("transaction must have postings from unique accounts")]
  DuplicateAccountPosting,

  #[error("account {0} not found")]
  AccountNotFound(i32),

  #[error(
    "insufficient funds in {account_id}: \
		available {current_balance},\
	 	attempted {attempted_delta}"
  )]
  InsufficientFunds {
    account_id: i32,
    current_balance: Decimal,
    attempted_delta: Decimal,
  },

  #[error("Transaction #{0} was not found")]
  TransactionNotFound(i32),

  #[error("Transaction #{0} has already been reversed")]
  AlreadyReversed(i32),

  #[error("Transaction #{0} is itself a reversal and cannot be reversed")]
  CannotReverseReversal(i32),

  #[error("Idempotency key replayed with mismatched request payload")]
  IdempotencyPayloadMismatch,

  #[error("database error: {0}")]
  DatabaseError(String),

  #[error("network error: {0}")]
  NetworkError(String),

  #[error("configuration error: {0}")]
  ConfigurationError(String),
}

impl From<sqlx::Error> for LedgerError {
  fn from(value: Error) -> Self {
    Self::DatabaseError(value.to_string())
  }
}

impl From<std::io::Error> for LedgerError {
  fn from(value: std::io::Error) -> Self {
    Self::NetworkError(value.to_string())
  }
}

impl From<envconfig::Error> for LedgerError {
  fn from(value: envconfig::Error) -> Self {
    Self::ConfigurationError(value.to_string())
  }
}

#[derive(Debug)]
pub enum ApiError {
  BadRequest(String),
  NotFound(String),
  UnprocessableEntity(String),
  InternalServerError,
  Unauthorized(String),
  Conflict(String),
  Forbidden(String),
}

impl From<LedgerError> for ApiError {
  fn from(err: LedgerError) -> Self {
    match err {
      LedgerError::InsufficientPostings(_)
      | LedgerError::ZeroAmountPosting
      | LedgerError::UnbalancedTransaction(_)
      | LedgerError::DuplicateAccountPosting
      | LedgerError::CannotReverseReversal(_) => ApiError::BadRequest(err.to_string()),
      LedgerError::AccountNotFound(_) | LedgerError::TransactionNotFound(_) => {
        ApiError::NotFound(err.to_string())
      }
      LedgerError::InsufficientFunds { .. } | 
      LedgerError::AlreadyReversed(_) | 
      LedgerError::IdempotencyPayloadMismatch => {
        ApiError::UnprocessableEntity(err.to_string())
      }
      LedgerError::DatabaseError(err)
      | LedgerError::NetworkError(err)
      | LedgerError::ConfigurationError(err) => {
        eprintln!("Internal error: {err:?}");
        ApiError::InternalServerError
      }
    }
  }
}

impl From<ValidationErrors> for ApiError {
  fn from(err: ValidationErrors) -> Self {
    Self::BadRequest(err.to_string())
  }
}

impl IntoResponse for ApiError {
  fn into_response(self) -> Response {
    let (status, message) = match self {
      ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
      ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
      ApiError::UnprocessableEntity(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg),
      ApiError::InternalServerError => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
      ),
      ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
      ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
      ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
    };

    let body = Json(json!({
      "error": message,
      "status": status.as_u16(),
    }));

    (status, body).into_response()
  }
}
