use axum::{extract::{Path, State}, http::StatusCode, Json, Router};
use axum::routing::post;
use serde::{Deserialize, Serialize};

use crate::app::{transactions, AppJson, AppState};
use crate::core::auth::AuthUser;
use crate::core::errors::AppResult;
use crate::core::ledger;
use crate::core::types::{IdempotencyKey, Note, ReverseCmd, TransferCmd};

// -------------- MODELS ----------------

#[derive(Serialize)]
struct TransactionResponse {
  transaction_id: i32,
}

#[derive(Deserialize)]
struct ReversalPayload {
  reason: Note,
}

// --------------- ROUTER ----------------

pub fn router() -> Router<AppState> {
  Router::new()
    .route("/", post(transfer))
    .route("/{id}/reverse", post(reverse))
}

// ---------------- ROUTES ------------------

async fn transfer(
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

async fn reverse(
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