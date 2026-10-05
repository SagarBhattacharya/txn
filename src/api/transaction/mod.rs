use crate::api::AppState;
use axum::Router;
use axum::routing::post;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationError};

mod routes;

#[derive(Debug, Deserialize, Validate)]
#[validate(schema(function = "validate_distinct_accounts"))]
pub struct TransferRequest {
  pub source_account_id: i32,
  pub destination_account_id: i32,

  #[validate(custom(function = "validate_positive_amount"))]
  pub amount: Decimal,

  #[validate(length(
    min = 1,
    max = 256,
    message = "Description must be between 1 and 256 characters"
  ))]
  pub description: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ReversalRequest {
  #[validate(custom(function = "validate_reason"))]
  pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct TransactionResponse {
  pub transaction_id: i32,
}

pub(crate) fn transaction_router() -> Router<AppState> {
  Router::new()
    .route("/", post(routes::transfer_funds))
    .route("/{id}/reverse", post(routes::reverse_transaction))
}

fn validate_positive_amount(amount: &Decimal) -> Result<(), ValidationError> {
  if *amount <= Decimal::ZERO {
    let mut err = ValidationError::new("invalid_amount");
    err.message = Some("Transfer amount must be strictly greater than zero".into());
    return Err(err);
  }
  Ok(())
}

fn validate_distinct_accounts(req: &TransferRequest) -> Result<(), ValidationError> {
  if req.source_account_id == req.destination_account_id {
    let mut err = ValidationError::new("identical_accounts");
    err.message = Some("Source and destination accounts must be distinct".into());
    return Err(err);
  }
  Ok(())
}

fn validate_reason(reason: &str) -> Result<(), ValidationError> {
  let len = reason.trim().chars().count();
  if !(3..=256).contains(&len) {
    let mut err = ValidationError::new("invalid_reason_length");
    err.message = Some("Reversal reason must be between 3 and 256 non-empty characters".into());
    return Err(err);
  }
  Ok(())
}
