use crate::api::AppState;
use crate::db::schemas::{Account, AccountType};
use axum::Router;
use axum::routing::{get, post};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationError};

mod routes;

#[derive(Debug, Deserialize, Validate)]
pub struct AccountCreateRequest {
  #[validate(custom(function = "validate_name"))]
  pub name: String,

  pub account_type: AccountType,
}

#[derive(Debug, Serialize)]
pub struct AccountResponse {
  pub id: i32,
  pub name: String,
  pub account_type: AccountType,
  pub owner_id: i32,
}

impl From<Account> for AccountResponse {
  fn from(acc: Account) -> Self {
    Self {
      id: acc.id,
      name: acc.name,
      account_type: acc.account_type,
      owner_id: acc.owner_id,
    }
  }
}

#[derive(Debug, Serialize)]
pub struct BalanceResponse {
  pub account_id: i32,
  pub balance: Decimal,
  pub owner_id: i32,
}

pub(crate) fn account_router() -> Router<AppState> {
  Router::new()
    .route("/", post(routes::create_account))
    .route("/", get(routes::get_all_accounts))
    .route("/{id}", get(routes::get_account_details))
    .route("/{id}/balance", get(routes::get_account_balance))
}

fn validate_name(name: &str) -> Result<(), ValidationError> {
  let len = name.trim().chars().count();
  if !(1..=128).contains(&len) {
    let mut err = ValidationError::new("invalid_account_name");
    err.message = Some("Account name must be between 1 and 128 non-empty characters".into());
    return Err(err);
  }
  Ok(())
}