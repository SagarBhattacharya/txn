use axum::{
  Json,
  http::StatusCode,
  response::{IntoResponse, Response},
};
use rust_decimal::Decimal;
use serde_json::json;

pub type AppResult<T> = Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
  // 400 Bad Request
  #[error("{0}")]
  BadRequest(String),

  #[error("Transaction must contain at least 2 postings, got {0}")]
  InsufficientPostings(usize),

  #[error("Posting amount cannot be zero")]
  ZeroAmountPosting,

  #[error("Transaction does not balance: delta is {0}")]
  UnbalancedTransaction(Decimal),

  #[error("Duplicate account posting in transaction")]
  DuplicateAccountPosting,

  #[error("Cannot reverse a reversal transaction: {0}")]
  CannotReverseReversal(i32),

  // 401 Unauthorized
  #[error("Unauthorized: {0}")]
  Unauthorized(String),

  // 403 Forbidden
  #[error("{0}")]
  Forbidden(String),

  // 404 Not Found
  #[error("{0}")]
  NotFound(String),

  #[error("Account with id {0} not found")]
  AccountNotFound(i32),

  #[error("Transaction with id {0} not found")]
  TransactionNotFound(i32),

  // 409 Conflict
  #[error("{0}")]
  Conflict(String),

  // 422 Unprocessable Entity
  #[error("Insufficient funds in account {account_id}: required {required}, available {available}")]
  InsufficientFunds {
    account_id: i32,
    required: Decimal,
    available: Decimal,
  },

  #[error("Transaction has already been reversed")]
  AlreadyReversed,

  #[error("Idempotency key replayed with mismatched request payload")]
  IdempotencyPayloadMismatch,

  // 500 Internal Server Error
  #[error("Database error: {0}")]
  Database(sqlx::Error),

  #[error("Internal server error: {0}")]
  Internal(String),
}

impl Error {
  pub fn status(&self) -> StatusCode {
    match self {
      Self::BadRequest(_)
      | Self::InsufficientPostings(_)
      | Self::ZeroAmountPosting
      | Self::UnbalancedTransaction(_)
      | Self::DuplicateAccountPosting
      | Self::CannotReverseReversal(_) => StatusCode::BAD_REQUEST,

      Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
      Self::Forbidden(_) => StatusCode::FORBIDDEN,

      Self::NotFound(_) | Self::AccountNotFound(_) | Self::TransactionNotFound(_) => {
        StatusCode::NOT_FOUND
      }

      Self::Conflict(_) => StatusCode::CONFLICT,

      Self::InsufficientFunds { .. } | Self::AlreadyReversed | Self::IdempotencyPayloadMismatch => {
        StatusCode::UNPROCESSABLE_ENTITY
      }

      Self::Database(_) | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
  }
}

impl From<sqlx::Error> for Error {
  fn from(e: sqlx::Error) -> Self {
    if let Some(db) = e.as_database_error() {
      match db.constraint() {
        Some("users_username_key") => {
          return Error::Conflict("Username already taken".into());
        }
        Some("uq_accounts_owner_name") => {
          return Error::Conflict("Account name already exists for this user".into());
        }
        Some("uq_transactions_single_reversal") => {
          return Error::AlreadyReversed;
        }
        _ => {}
      }
    }
    Error::Database(e)
  }
}

impl IntoResponse for Error {
  fn into_response(self) -> Response {
    let status = self.status();

    // 500 errors log details internally and emit generic messages to users
    let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
      tracing::error!(
        error = ?self,
        "unhandled internal server error during request execution"
      );
      "Internal server error".to_string()
    } else {
      self.to_string()
    };

    let body = Json(json!({
      "error": message,
      "status": status.as_u16(),
    }));

    (status, body).into_response()
  }
}
