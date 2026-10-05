use axum::{
	extract::{rejection::JsonRejection, FromRequest, Request},
	Json,
};
use validator::Validate;
use crate::ApiError;

pub struct ValidatedJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidatedJson<T>
where
	S: Send + Sync,
	Json<T>: FromRequest<S, Rejection = JsonRejection>,
	T: Validate + 'static,
{
	type Rejection = ApiError;

	async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
		let Json(value) = Json::<T>::from_request(req, state)
			.await
			.map_err(|e| ApiError::BadRequest(e.to_string()))?;

		value.validate()?;
		Ok(ValidatedJson(value))
	}
}