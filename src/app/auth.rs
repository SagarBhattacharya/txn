use crate::app::AppState;
use crate::core::errors::{AppResult, Error};
use crate::db::rows::User;
use argon2::password_hash::phc::PasswordHash;
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

// Precomputed valid Argon2id hash for constant-time dummy verification
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
  hash_password("dummy_constant_time_pass_for_timing_mitigation")
    .expect("Failed to initialize dummy hash")
});

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
  pub sub: i32,
  pub exp: usize,
  pub iat: usize,
}

#[derive(Clone)]
pub struct JwtKeys {
  pub encoding: EncodingKey,
  pub decoding: DecodingKey,
}

impl JwtKeys {
  pub fn new(secret: &str) -> Self {
    Self {
      encoding: EncodingKey::from_secret(secret.as_bytes()),
      decoding: DecodingKey::from_secret(secret.as_bytes()),
    }
  }

  pub fn issue(&self, user_id: i32) -> AppResult<String> {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = Claims {
      sub: user_id,
      iat: now,
      exp: now + (24 * 3600), // 24 hours
    };

    jsonwebtoken::encode(&Header::default(), &claims, &self.encoding)
      .map_err(|e| Error::Internal(format!("Failed to issue JWT: {e}")))
  }

  pub fn verify(&self, token: &str) -> AppResult<Claims> {
    let validation = Validation::default();

    jsonwebtoken::decode::<Claims>(token, &self.decoding, &validation)
      .map(|data| data.claims)
      .map_err(|e| Error::Unauthorized(format!("Invalid or expired token: {e}")))
  }
}

pub fn hash_password(password: &str) -> AppResult<String> {
  Argon2::default()
    .hash_password(password.as_bytes())
    .map(|h| h.to_string())
    .map_err(|e| Error::Internal(format!("Password hashing failed: {e}")))
}

pub fn verify_login(user: Option<&User>, password: &str) -> AppResult<bool> {
  let (target_hash, user_exists) = match user {
    Some(u) => (u.password_hash.as_str(), true),
    None => (DUMMY_HASH.as_str(), false),
  };

  let parsed_hash = PasswordHash::new(target_hash)
    .map_err(|_| Error::Internal("Failed to create password hash".to_string()))?;

  let matches = Argon2::default()
    .verify_password(password.as_bytes(), &parsed_hash)
    .is_ok();

  Ok(user_exists && matches)
}

#[derive(Debug, Clone, Copy)]
pub struct AuthUser {
  pub id: i32,
}

// Extractor: Allows handlers to accept `user: AuthUser` directly
impl<S> FromRequestParts<S> for AuthUser
where
  S: Send + Sync,
{
  type Rejection = Error;

  async fn from_request_parts(
    parts: &mut axum::http::request::Parts,
    _state: &S,
  ) -> Result<Self, Self::Rejection> {
    parts
      .extensions
      .get::<AuthUser>()
      .copied()
      .ok_or_else(|| Error::Unauthorized("Missing authentication credentials".into()))
  }
}

// Middleware: Validates Bearer token and inserts AuthUser into request extensions
pub async fn auth_middleware(
  State(state): State<AppState>,
  mut req: Request,
  next: Next,
) -> Result<Response, Error> {
  let auth_header = req
    .headers()
    .get(AUTHORIZATION)
    .and_then(|header| header.to_str().ok())
    .ok_or_else(|| Error::Unauthorized("Missing Authorization header".into()))?;

  let token = auth_header
    .strip_prefix("Bearer ")
    .ok_or_else(|| Error::Unauthorized("Malformed Authorization header".into()))?;

  let claims = state.jwt.verify(token)?;
  req.extensions_mut().insert(AuthUser { id: claims.sub });

  Ok(next.run(req).await)
}
