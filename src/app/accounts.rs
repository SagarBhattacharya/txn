use axum::{
  Json,
  extract::{Path, State},
  http::StatusCode,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::app::auth::AuthUser;
use crate::app::{AppJson, AppState};
use crate::core::errors::AppResult;
use crate::core::ledger;
use crate::core::types::AccountName;
use crate::db::queries::Query;
use crate::db::rows::{Account, AccountActivity, AccountType};

#[derive(Debug, Deserialize)]
pub struct CreateAccountPayload {
  pub name: AccountName,
  pub account_type: AccountType,
}

#[derive(Debug, Serialize)]
pub struct BalanceResponse {
  pub account_id: i32,
  pub balance: Decimal,
}

pub async fn create_account(
  user: AuthUser,
  State(state): State<AppState>,
  AppJson(payload): AppJson<CreateAccountPayload>,
) -> AppResult<(StatusCode, Json<Account>)> {
  let account = Query::create_account(
    &state.pool,
    user.id,
    payload.name.as_str(),
    payload.account_type,
  )
  .await?;

  Ok((StatusCode::CREATED, Json(account)))
}

pub async fn list_accounts(
  user: AuthUser,
  State(state): State<AppState>,
) -> AppResult<Json<Vec<Account>>> {
  let accounts = Query::get_accounts_by_owner(&state.pool, user.id).await?;

  Ok(Json(accounts))
}

pub async fn get_account(
  user: AuthUser,
  Path(id): Path<i32>,
  State(state): State<AppState>,
) -> AppResult<Json<Account>> {
  let account = ledger::owned_account(&state.pool, id, user.id).await?;
  Ok(Json(account))
}

pub async fn get_balance(
  user: AuthUser,
  Path(id): Path<i32>,
  State(state): State<AppState>,
) -> AppResult<Json<BalanceResponse>> {
  ledger::owned_account(&state.pool, id, user.id).await?;
  let balance = Query::get_balance(&state.pool, id).await?;

  Ok(Json(BalanceResponse {
    account_id: id,
    balance,
  }))
}

pub async fn get_activity(
  user: AuthUser, 
  Path(id): Path<i32>, 
  State(state): State<AppState>
) -> AppResult<Json<Vec<AccountActivity>>> {
  ledger::owned_account(&state.pool, id, user.id).await?;
  Ok(Json(Query::get_account_activity(&state.pool, id).await?))
}
