use crate::api::AppState;
use axum::Router;
use axum::routing::post;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

mod routes;

#[derive(Debug, Deserialize)]
pub struct TransferRequest {
  pub source_account_id: i32,
  pub destination_account_id: i32,
  pub amount: Decimal,
  pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct ReversalRequest {
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
