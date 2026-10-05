use axum::Router;
use axum::routing::post;
use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationError};
use crate::api::AppState;

mod routes;

#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
	#[validate(custom(function = "validate_username"))]
	pub username: String,

	#[validate(length(
		min = 8,
		max = 128,
		message = "Password must be between 8 and 128 characters long"
	))]
	pub password: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct LoginRequest {
	#[validate(length(min = 1, max = 64, message = "Username cannot be empty"))]
	pub username: String,

	#[validate(length(min = 1, max = 128, message = "Password cannot be empty"))]
	pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
	pub user_id: i32,
	pub username: String,
	pub token: String,
}

pub fn auth_router() -> Router<AppState> {
	Router::new()
		.route("/register", post(routes::register))
		.route("/login", post(routes::login))
}

fn validate_username(username: &str) -> Result<(), ValidationError> {
	let trimmed = username.trim();
	if trimmed.len() < 3 || trimmed.len() > 64 {
		let mut err = ValidationError::new("invalid_username_length");
		err.message = Some("Username must be between 3 and 64 characters".into());
		return Err(err);
	}

	// Allow only alphanumeric characters and underscores
	if !trimmed.chars().all(|c| c.is_alphanumeric() || c == '_') {
		let mut err = ValidationError::new("invalid_username_format");
		err.message = Some("Username may only contain letters, numbers, and underscores".into());
		return Err(err);
	}

	Ok(())
}