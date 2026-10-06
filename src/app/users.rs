use axum::http::StatusCode;
use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::app::auth::{JwtKeys, hash_password, verify_login};
use crate::app::{AppJson, AppState};
use crate::core::errors::{AppResult, Error};
use crate::core::types::{Password, Username};
use crate::db::queries::Query;

#[derive(Deserialize)]
pub struct AuthPayload {
  pub username: Username,
  pub password: Password,
}

#[derive(Serialize)]
pub struct AuthResponse {
  pub user_id: i32,
  pub username: String,
  pub token: String,
}

fn auth_response(user_id: i32, user_name: &str, jwt: &JwtKeys) -> AppResult<Json<AuthResponse>> {
  let token = jwt.issue(user_id)?;
  Ok(Json(AuthResponse {
    user_id,
    username: user_name.into(),
    token,
  }))
}

pub async fn register(
  State(state): State<AppState>,
  AppJson(payload): AppJson<AuthPayload>,
) -> AppResult<(StatusCode, Json<AuthResponse>)> {
  let hash = hash_password(payload.password.expose())?;
  let user = Query::create_user(&state.pool, payload.username.as_str(), &hash).await?;

  let res = auth_response(user.id, &user.username, &state.jwt)?;
  Ok((StatusCode::CREATED, res))
}

pub async fn login(
  State(state): State<AppState>,
  AppJson(payload): AppJson<AuthPayload>,
) -> AppResult<Json<AuthResponse>> {
  let user = Query::get_user_by_username(&state.pool, payload.username.as_str()).await?;

  if !verify_login(user.as_ref(), payload.password.expose())? {
    return Err(Error::Unauthorized("Invalid username or password".into()));
  }

  let user = user.expect("verified user must exist");
  auth_response(user.id, &user.username, &state.jwt)
}
