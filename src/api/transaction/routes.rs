use super::{ReversalRequest, TransactionResponse, TransferRequest};
use crate::api::AppState;
use crate::models::draft::{PostingDraft, TransactionDraft};
use crate::{ApiError, LedgerError, LedgerResult};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use rust_decimal::Decimal;

pub async fn transfer_funds(
  headers: HeaderMap,
  State(state): State<AppState>,
  Json(payload): Json<TransferRequest>,
) -> Result<(StatusCode, Json<TransactionResponse>), ApiError> {
  let idempotency_key = extract_idempotency_key(&headers)?;
  let draft = TransactionDraft::from_transfer(payload, idempotency_key)?;
  let txn_id = state.repo.record_transaction(draft).await?;

  Ok((
    StatusCode::CREATED,
    Json(TransactionResponse {
      transaction_id: txn_id,
    }),
  ))
}

pub async fn reverse_transaction(
  headers: HeaderMap,
  State(state): State<AppState>,
  Path(id): Path<i32>,
  Json(payload): Json<ReversalRequest>,
) -> Result<(StatusCode, Json<TransactionResponse>), ApiError> {
  let idempotency_key = extract_idempotency_key(&headers)?;
  let txn_id = state
    .repo
    .reverse_transaction(id, &payload.reason, idempotency_key)
    .await?;

  Ok((
    StatusCode::CREATED,
    Json(TransactionResponse {
      transaction_id: txn_id,
    }),
  ))
}

impl TransactionDraft {
  fn from_transfer(req: TransferRequest, key: String) -> LedgerResult<Self> {
    // Enforce positive amount for simple transfers
    if req.amount <= Decimal::ZERO {
      return Err(LedgerError::ZeroAmountPosting);
    }

    // 1. Money leaving source (negative delta)
    let source_posting = PostingDraft::new(req.source_account_id, -req.amount)?;

    // 2. Money entering destination (positive delta)
    let dest_posting = PostingDraft::new(req.destination_account_id, req.amount)?;

    // 3. Construct logic draft (enforces distinct accounts, sum == 0, etc.)
    TransactionDraft::new(req.description, key, vec![source_posting, dest_posting])
  }
}

fn extract_idempotency_key(headers: &HeaderMap) -> Result<String, ApiError> {
  headers
    .get("idempotency-key")
    .and_then(|val| val.to_str().ok())
    .map(|s| s.to_string())
    .ok_or_else(|| ApiError::BadRequest("idempotency-key not found in headers".into()))
}
