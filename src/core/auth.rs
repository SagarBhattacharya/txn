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

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::{Duration, Utc};

  fn test_user(password_hash: String) -> User {
    User {
      id: 42,
      username: "alice".to_string(),
      password_hash,
      created_at: Utc::now(),
    }
  }

  #[test]
  fn test_jwt_issue_and_verify_cycle() {
    let keys = JwtKeys::new("super-secure-secret-key-at-least-32-bytes-long");
    let user_id = 101;

    let token = keys.issue(user_id).expect("issuing JWT should succeed");
    let claims = keys.verify(&token).expect("verifying valid JWT should succeed");

    assert_eq!(claims.sub, user_id);
    assert!(claims.exp > claims.iat);
  }

  #[test]
  fn test_jwt_rejects_wrong_secret() {
    let keys_signer = JwtKeys::new("signing-secret-key-at-least-32-bytes-long");
    let keys_verifier = JwtKeys::new("different-secret-key-at-least-32-bytes-long");

    let token = keys_signer.issue(101).expect("issuing token should succeed");
    let result = keys_verifier.verify(&token);

    assert!(matches!(result, Err(Error::Unauthorized(_))));
  }

  #[test]
  fn test_jwt_rejects_expired_token() {
    let secret = "test-secret-key-at-least-32-bytes-long";
    let keys = JwtKeys::new(secret);

    // Manually construct an expired token
    let now = (Utc::now() - Duration::hours(48)).timestamp() as usize;
    let expired_claims = Claims {
      sub: 101,
      iat: now - 3600,
      exp: now,
    };

    let token = jsonwebtoken::encode(
      &Header::default(),
      &expired_claims,
      &EncodingKey::from_secret(secret.as_bytes()),
    )
      .expect("encoding expired token should succeed");

    let result = keys.verify(&token);
    assert!(matches!(result, Err(Error::Unauthorized(_))));
  }

  #[test]
  fn test_password_hash_and_verify_login_success() {
    let password = "correct_horse_battery_staple";
    let hash = hash_password(password).expect("hashing password should succeed");
    let user = test_user(hash);

    let auth_success = verify_login(Some(&user), password).expect("verify should succeed");
    assert!(auth_success);
  }

  #[test]
  fn test_verify_login_wrong_password() {
    let hash = hash_password("valid_password").expect("hashing should succeed");
    let user = test_user(hash);

    let auth_failed = verify_login(Some(&user), "wrong_password").expect("verify should execute");
    assert!(!auth_failed);
  }

  #[test]
  fn test_verify_login_user_not_found_runs_dummy_hash() {
    // Exercises the DUMMY_HASH branch to protect against timing attacks
    let auth_failed = verify_login(None, "some_attempted_password").expect("verify should execute");
    assert!(!auth_failed);
  }

  #[tokio::test]
  async fn test_auth_user_extractor_success() {
    let mut req = axum::http::Request::builder()
      .body(())
      .expect("request build");

    req.extensions_mut().insert(AuthUser { id: 777 });

    let (mut parts, _) = req.into_parts();
    let auth_user = AuthUser::from_request_parts(&mut parts, &())
      .await
      .expect("extractor should succeed when extension is present");

    assert_eq!(auth_user.id, 777);
  }

  #[tokio::test]
  async fn test_auth_user_extractor_missing() {
    let req = axum::http::Request::builder()
      .body(())
      .expect("request build");

    let (mut parts, _) = req.into_parts();
    let result = AuthUser::from_request_parts(&mut parts, &()).await;

    assert!(matches!(result, Err(Error::Unauthorized(_))));
  }
}