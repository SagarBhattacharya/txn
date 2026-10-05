use axum::{
	extract::{FromRequestParts, Request, State},
	http::{header, request::Parts, StatusCode},
	middleware::Next,
	response::Response,
};

use crate::api::AppState;
use crate::auth::verify_jwt;
use crate::ApiError;

#[derive(Clone, Debug)]
pub struct AuthUser {
	pub id: i32,
	pub username: String,
}

/// 1. Middleware function that validates token and injects AuthUser into request extensions
pub async fn auth_middleware(
	State(state): State<AppState>,
	mut req: Request,
	next: Next,
) -> Result<Response, ApiError> {
	let auth_header = req
		.headers()
		.get(header::AUTHORIZATION)
		.and_then(|val| val.to_str().ok())
		.ok_or_else(|| ApiError::Unauthorized("Missing Authorization header".into()))?;

	let token = auth_header
		.strip_prefix("Bearer ")
		.ok_or_else(|| ApiError::Unauthorized("Authorization header must use Bearer scheme".into()))?;

	// Validate claims using the app's secret
	let claims = verify_jwt(token, state.jwt_secret.as_bytes())?;

	// Attach user data to the request so downstream extractors and handlers can read it
	req.extensions_mut().insert(AuthUser {
		id: claims.sub,
		username: claims.username,
	});

	Ok(next.run(req).await)
}

/// 2. Custom Extractor: Allows route handlers to simply declare `user: AuthUser`
impl<S> FromRequestParts<S> for AuthUser
where
	S: Send + Sync,
{
	type Rejection = (StatusCode, &'static str);

	async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
		parts
			.extensions
			.get::<AuthUser>()
			.cloned()
			.ok_or((StatusCode::UNAUTHORIZED, "Unauthenticated user context"))
	}
}