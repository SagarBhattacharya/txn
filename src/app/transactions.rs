use axum::{
  Json,
  extract::{Path, State},
  http::StatusCode,
};
use serde::Serialize;

use crate::app::auth::AuthUser;
use crate::app::{AppJson, AppState};
use crate::core::errors::AppResult;
use crate::core::ledger;
use crate::core::types::{IdempotencyKey, Note, ReverseCmd, TransferCmd};

#[derive(Serialize)]
pub struct TransactionResponse {
  pub transaction_id: i32,
}

#[derive(serde::Deserialize)]
pub struct ReversalPayload {
  pub reason: Note,
}

pub async fn transfer(
  user: AuthUser,
  key: IdempotencyKey,
  State(state): State<AppState>,
  AppJson(cmd): AppJson<TransferCmd>,
) -> AppResult<(StatusCode, Json<TransactionResponse>)> {
  let id = ledger::transfer(&state.pool, user.id, &key, &cmd).await?;
  Ok((
    StatusCode::CREATED,
    Json(TransactionResponse { transaction_id: id }),
  ))
}

pub async fn reverse(
  user: AuthUser,
  key: IdempotencyKey,
  Path(target): Path<i32>,
  State(state): State<AppState>,
  AppJson(payload): AppJson<ReversalPayload>,
) -> AppResult<(StatusCode, Json<TransactionResponse>)> {
  let cmd = ReverseCmd {
    target,
    reason: payload.reason,
  };
  let id = ledger::reverse(&state.pool, user.id, &key, &cmd).await?;
  Ok((
    StatusCode::CREATED,
    Json(TransactionResponse { transaction_id: id }),
  ))
}
