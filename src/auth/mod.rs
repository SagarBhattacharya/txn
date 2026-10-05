pub mod middleware;

use std::sync::LazyLock;
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use argon2::password_hash::phc::PasswordHash;
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::ApiError;

// Precomputed valid Argon2id hash for constant-time dummy verification
static DUMMY_ARGON2_HASH: LazyLock<String> = LazyLock::new(|| {
	hash_password("dummy_constant_time_pass_for_timing_mitigation")
		.expect("Failed to initialize dummy hash")
});

pub fn verify_dummy_password(password: &str) {
	let _ = verify_password(password, &DUMMY_ARGON2_HASH);
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
	pub sub: i32,
	pub username: String,
	pub exp: i64,
	pub iat: i64,
}

pub fn hash_password(password: &str) -> Result<String, ApiError> {
	let argon2 = Argon2::default();

	argon2
		.hash_password(password.as_bytes())
		.map(|hash| hash.to_string())
		.map_err(|_| ApiError::InternalServerError)
}

pub fn verify_password(password: &str, password_hash: &str) -> Result<bool, ApiError> {
	let parsed_hash = PasswordHash::new(password_hash)
		.map_err(|_| ApiError::InternalServerError)?;

	Ok(Argon2::default()
		.verify_password(password.as_bytes(), &parsed_hash)
		.is_ok())
}

pub fn create_jwt(user_id: i32, username: &str, secret: &[u8]) -> Result<String, ApiError> {
	let now = Utc::now();
	let claims = Claims {
		sub: user_id,
		username: username.to_string(),
		iat: now.timestamp(),
		exp: (now + Duration::hours(24)).timestamp(),
	};

	encode(&Header::default(), &claims, &EncodingKey::from_secret(secret))
		.map_err(|_| ApiError::InternalServerError)
}

pub fn verify_jwt(token: &str, secret: &[u8]) -> Result<Claims, ApiError> {
	decode::<Claims>(
		token,
		&DecodingKey::from_secret(secret),
		&Validation::default(),
	)
		.map(|token_data| token_data.claims)
		.map_err(|_| ApiError::Unauthorized("Invalid or expired token".into()))
}