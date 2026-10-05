use super::*;
use crate::api::AppState;
use crate::api::extractors::ValidatedJson;
use crate::auth::*;
use crate::ApiError;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

pub async fn register(
	State(state): State<AppState>,
	ValidatedJson(payload): ValidatedJson<RegisterRequest>,
) -> Result<impl IntoResponse, ApiError> {
	let username = payload.username.trim();
	let password_hash = hash_password(&payload.password)?;

	let user = state
		.repo
		.create_user(username, &password_hash)
		.await?
		.ok_or_else(|| ApiError::Conflict("Username already taken".into()))?;

	let token = create_jwt(user.id, &user.username, state.jwt_secret.as_bytes())?;
	Ok((
		StatusCode::CREATED,
		Json(AuthResponse {
			user_id: user.id,
			username: user.username,
			token,
		}),
	))
}

pub async fn login(
	State(state): State<AppState>,
	ValidatedJson(payload): ValidatedJson<LoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
	let maybe_user = state.repo.get_user_by_username(&payload.username).await?;

	let is_valid = match maybe_user {
		Some(ref user) => verify_password(&payload.password, &user.password_hash)?,
		None => {
			// Burn identical CPU cycles to prevent timing enumeration attacks
			verify_dummy_password(&payload.password);
			false
		}
	};

	if !is_valid {
		return Err(ApiError::Unauthorized("Invalid username or password".into()));
	}

	let user = maybe_user.unwrap();
	let token = create_jwt(user.id, &user.username, state.jwt_secret.as_bytes())?;

	Ok((
		StatusCode::OK,
		Json(AuthResponse {
			user_id: user.id,
			username: user.username,
			token,
		}),
	))
}