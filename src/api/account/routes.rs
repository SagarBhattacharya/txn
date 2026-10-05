use super::{AccountCreateRequest, AccountResponse, BalanceResponse};
use crate::ApiError;
use crate::api::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use crate::api::extractors::ValidatedJson;
use crate::auth::middleware::AuthUser;

pub async fn create_account(
  user: AuthUser,
  State(state): State<AppState>,
  ValidatedJson(payload): ValidatedJson<AccountCreateRequest>,
) -> Result<(StatusCode, Json<AccountResponse>), ApiError> {
  let result = state
    .repo
    .create_account(user.id, payload.name.trim(), payload.account_type)
    .await?
    .ok_or_else(|| {
      ApiError::Conflict(format!(
        "Account with name '{}' already exists for this user",
        payload.name.trim()
      ))
    })?;

  Ok((StatusCode::CREATED, Json(AccountResponse::from(result))))
}

pub async fn get_account_details(
  user: AuthUser,
  State(state): State<AppState>,
  Path(id): Path<i32>,
) -> Result<Json<AccountResponse>, ApiError> {
  let account = state
    .repo
    .get_account_by_id(id)
    .await?
    .ok_or_else(|| ApiError::NotFound(
      format!("Account with id {id} not found")
    ))?;

  if user.id != account.owner_id {
    return Err(ApiError::Forbidden("Access Denied".into()));
  }

  Ok(Json(AccountResponse::from(account)))
}

pub async fn get_account_balance(
  user: AuthUser,
  State(state): State<AppState>,
  Path(id): Path<i32>,
) -> Result<Json<BalanceResponse>, ApiError> {
  let account = state
    .repo
    .get_account_by_id(id)
    .await?
    .ok_or_else(|| ApiError::NotFound(
      format!("Account with id {id} not found")
    ))?;

  if user.id != account.owner_id {
    return Err(ApiError::Forbidden("Access Denied".into()));
  }

  let balance = state.repo.get_account_balance(id).await?;

  Ok(Json(BalanceResponse {
    account_id: id,
    balance,
    owner_id: user.id,
  }))
}

pub async fn get_all_accounts(
  user: AuthUser,
  State(state): State<AppState>,
) -> Result<Json<Vec<AccountResponse>>, ApiError> {
  let accounts = state.repo.get_accounts_by_owner(user.id).await?;
  Ok(Json(accounts.into_iter().map(AccountResponse::from).collect()))
}