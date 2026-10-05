use crate::api::AppState;
use crate::db::schemas::AccountType;
use axum::Router;
use axum::routing::{get, post};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

mod routes;

#[derive(Debug, Deserialize)]
pub struct AccountCreateRequest {
  pub name: String,
  pub account_type: AccountType,
}

#[derive(Debug, Serialize)]
pub struct AccountResponse {
  pub id: i32,
  pub name: String,
  pub account_type: AccountType,
}

#[derive(Debug, Serialize)]
pub struct BalanceResponse {
  pub account_id: i32,
  pub balance: Decimal,
}

pub(crate) fn account_router() -> Router<AppState> {
  Router::new()
    .route("/", post(routes::create_account))
    .route("/{id}", get(routes::get_account_details))
    .route("/{id}/balance", get(routes::get_account_balance))
}
