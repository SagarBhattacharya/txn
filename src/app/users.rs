use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router, extract::State};
use serde::{Deserialize, Serialize};

use crate::app::{AppJson, AppState};
use crate::core::auth::{hash_password, verify_login};
use crate::core::errors::{AppResult, Error};
use crate::core::types::{Password, Username};
use crate::db;

// -------------- MODELS ----------------

#[derive(Deserialize)]
struct AuthPayload {
  username: Username,
  password: Password,
}

#[derive(Serialize)]
struct AuthResponse {
  user_id: i32,
  username: String,
  token: String,
}

// --------------- ROUTER ----------------

pub fn router() -> Router<AppState> {
  Router::new()
    .route("/register", post(register))
    .route("/login", post(login))
}

// ---------------- ROUTES ------------------

async fn register(
  State(state): State<AppState>,
  AppJson(payload): AppJson<AuthPayload>,
) -> AppResult<(StatusCode, Json<AuthResponse>)> {
  let hash = hash_password(payload.password.expose())?;
  let user = db::create_user(&state.pool, payload.username.as_str(), &hash).await?;

  let token = state.jwt.issue(user.id)?;
  Ok((
    StatusCode::CREATED,
    Json(AuthResponse {
      user_id: user.id,
      username: user.username,
      token,
    }),
  ))
}

async fn login(
  State(state): State<AppState>,
  AppJson(payload): AppJson<AuthPayload>,
) -> AppResult<Json<AuthResponse>> {
  let user = db::get_user_by_username(&state.pool, payload.username.as_str()).await?;

  if !verify_login(user.as_ref(), payload.password.expose())? {
    return Err(Error::Unauthorized("Invalid username or password".into()));
  }

  let user = user.expect("verified user must exist");
  let token = state.jwt.issue(user.id)?;
  Ok(Json(AuthResponse {
    user_id: user.id,
    username: user.username,
    token,
  }))
}
