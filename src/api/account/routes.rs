use super::{AccountCreateRequest, AccountResponse, BalanceResponse};
use crate::ApiError;
use crate::api::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

pub async fn create_account(
  State(state): State<AppState>,
  Json(payload): Json<AccountCreateRequest>,
) -> Result<(StatusCode, Json<AccountResponse>), ApiError> {
  let result = state
    .repo
    .create_account(payload.name.as_str(), payload.account_type)
    .await?;

  Ok((
    StatusCode::CREATED,
    Json(AccountResponse {
      id: result.id,
      name: result.name,
      account_type: result.account_type,
    }),
  ))
}

pub async fn get_account_details(
  State(state): State<AppState>,
  Path(id): Path<i32>,
) -> Result<Json<AccountResponse>, ApiError> {
  let result = state.repo.get_account_by_id(id).await?;
  if let Some(account) = result {
    Ok(Json(AccountResponse {
      id: account.id,
      name: account.name,
      account_type: account.account_type,
    }))
  } else {
    Err(ApiError::NotFound(format!(
      "Account with id {} not found",
      id
    )))
  }
}

pub async fn get_account_balance(
  State(state): State<AppState>,
  Path(id): Path<i32>,
) -> Result<Json<BalanceResponse>, ApiError> {
  let account = state.repo.get_account_by_id(id).await?;
  if account.is_none() {
    return Err(ApiError::NotFound(format!(
      "Account with id {} not found",
      id
    )));
  }

  let result = state.repo.get_account_balance(id).await?;
  Ok(Json(BalanceResponse {
    account_id: id,
    balance: result,
  }))
}
