use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use rust_decimal::Decimal;

use super::{ReversalRequest, TransactionResponse, TransferRequest};
use crate::api::extractors::ValidatedJson;
use crate::api::AppState;
use crate::auth::middleware::AuthUser;
use crate::models::draft::{PostingDraft, TransactionDraft};
use crate::{ApiError, LedgerError, LedgerResult};

pub async fn transfer_funds(
  user: AuthUser,
  headers: HeaderMap,
  State(state): State<AppState>,
  ValidatedJson(payload): ValidatedJson<TransferRequest>,
) -> Result<(StatusCode, Json<TransactionResponse>), ApiError> {
  // 1. Authorization: Verify caller owns the source account
  let source_account = state
    .repo
    .get_account_by_id(payload.source_account_id)
    .await?
    .ok_or_else(|| {
      ApiError::NotFound(format!(
        "Source account {} not found",
        payload.source_account_id
      ))
    })?;

  if source_account.owner_id != user.id {
    return Err(ApiError::Forbidden(
      "You are not authorized to transfer funds from this account".into(),
    ));
  }

  // 2. Verification: Ensure destination account exists before locking
  let _dest_account = state
    .repo
    .get_account_by_id(payload.destination_account_id)
    .await?
    .ok_or_else(|| {
      ApiError::NotFound(format!(
        "Destination account {} not found",
        payload.destination_account_id
      ))
    })?;

  // 3. Execution: Idempotency check & zero-sum ledger recording
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
  user: AuthUser,
  headers: HeaderMap,
  State(state): State<AppState>,
  Path(id): Path<i32>,
  ValidatedJson(payload): ValidatedJson<ReversalRequest>,
) -> Result<(StatusCode, Json<TransactionResponse>), ApiError> {
  // 1. Authorize: Single query checks if transaction exists AND user owns an involved account
  let is_party = state.repo.is_user_party_to_transaction(id, user.id).await?;
  
  if !is_party {
    // Return 404 if it doesn't exist, or 403 if it exists but caller is not a party
    let exists = state.repo.get_entries_by_transaction_id(id).await?.is_empty();
    if exists {
      return Err(ApiError::NotFound(format!("Transaction with id {id} not found")));
    }
    
    return Err(ApiError::Forbidden(
      "You are not authorized to reverse this transaction".into(),
    ));
  }

  // 2. Execute reversal via engine
  let idempotency_key = extract_idempotency_key(&headers)?;
  let txn_id = state
    .repo
    .reverse_transaction(id, payload.reason.trim(), idempotency_key)
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
    if req.amount <= Decimal::ZERO {
      return Err(LedgerError::ZeroAmountPosting);
    }

    let source_posting = PostingDraft::new(req.source_account_id, -req.amount)?;
    let dest_posting = PostingDraft::new(req.destination_account_id, req.amount)?;

    TransactionDraft::new(req.description, key, vec![source_posting, dest_posting])
  }
}

fn extract_idempotency_key(headers: &HeaderMap) -> Result<String, ApiError> {
  let key = headers
    .get("idempotency-key")
    .and_then(|val| val.to_str().ok())
    .map(|s| s.trim())
    .filter(|s| !s.is_empty())
    .ok_or_else(|| ApiError::BadRequest("idempotency-key not found in headers".into()))?;

  if key.len() > 128 {
    return Err(ApiError::BadRequest(
      "idempotency-key exceeds maximum length of 128 characters".into(),
    ));
  }

  Ok(key.to_string())
}