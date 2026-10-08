use axum::routing::{get, post};
use axum::{
  Json, Router,
  extract::{Path, State},
  http::StatusCode,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::app::{AppJson, AppState};
use crate::core::auth::AuthUser;
use crate::core::errors::AppResult;
use crate::core::ledger;
use crate::core::types::AccountName;
use crate::db;
use crate::db::rows::{Account, AccountActivity, AccountType};

// -------------- MODELS ----------------

#[derive(Debug, Deserialize)]
struct CreateAccountPayload {
  name: AccountName,
  account_type: AccountType,
}

#[derive(Debug, Serialize)]
struct BalanceResponse {
  account_id: i32,
  balance: Decimal,
}

// --------------- ROUTER ----------------

pub fn router() -> Router<AppState> {
  Router::new()
    .route("/", post(create_account))
    .route("/", get(list_accounts))
    .route("/{id}", get(get_account))
    .route("/{id}/balance", get(get_balance))
    .route("/{id}/transactions", get(get_activity))
}

// ---------------- ROUTES ------------------

async fn create_account(
  user: AuthUser,
  State(state): State<AppState>,
  AppJson(payload): AppJson<CreateAccountPayload>,
) -> AppResult<(StatusCode, Json<Account>)> {
  let account = db::create_account(
    &state.pool,
    user.id,
    payload.name.as_str(),
    payload.account_type,
  )
  .await?;

  Ok((StatusCode::CREATED, Json(account)))
}

async fn list_accounts(
  user: AuthUser,
  State(state): State<AppState>,
) -> AppResult<Json<Vec<Account>>> {
  let accounts = db::get_accounts_by_owner(&state.pool, user.id).await?;

  Ok(Json(accounts))
}

async fn get_account(
  user: AuthUser,
  Path(id): Path<i32>,
  State(state): State<AppState>,
) -> AppResult<Json<Account>> {
  let account = ledger::owned_account(&state.pool, id, user.id).await?;
  Ok(Json(account))
}

async fn get_balance(
  user: AuthUser,
  Path(id): Path<i32>,
  State(state): State<AppState>,
) -> AppResult<Json<BalanceResponse>> {
  ledger::owned_account(&state.pool, id, user.id).await?;
  let balance = db::get_balance(&state.pool, id).await?;

  Ok(Json(BalanceResponse {
    account_id: id,
    balance,
  }))
}

async fn get_activity(
  user: AuthUser,
  Path(id): Path<i32>,
  State(state): State<AppState>,
) -> AppResult<Json<Vec<AccountActivity>>> {
  ledger::owned_account(&state.pool, id, user.id).await?;
  Ok(Json(db::get_account_activity(&state.pool, id).await?))
}
