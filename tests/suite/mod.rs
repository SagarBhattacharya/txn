use axum::{
	Router,
	body::Body,
	http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use txn::db::rows::{AccountType, User};
use txn::{App, AppState, JwtKeys, db, hash_password};
use uuid::Uuid;

#[derive(Clone)]
pub struct TestHarness {
	pub pool: PgPool,
	pub jwt: Arc<JwtKeys>,
	pub app: Router,
}

impl TestHarness {
	pub async fn new(pool: PgPool) -> Self {
		let test_secret = "SwVIKb0FwGfQMGmJbLFgz10wY00zjgDjlklHlyyu7gI";
		let jwt = Arc::new(JwtKeys::new(test_secret));

		let state = AppState::new(pool.clone(), jwt.clone());
		let app = App::from_state(state)
			.expect("router creation should not fail")
			.router();

		Self { pool, jwt, app }
	}

	pub async fn create_user(&self, username: &str, password: &str) -> UserSession {
		let pass_hash = hash_password(password).expect("hash password");
		let user = db::create_user(&self.pool, username, &pass_hash)
			.await
			.expect("create user in db");
		let token = self.jwt.issue(user.id).expect("issue token");

		UserSession {
			user,
			token,
			harness: self.clone(),
		}
	}

	pub async fn request_raw(
		&self,
		method: Method,
		uri: &str,
		auth_token: Option<&str>,
		idempotency_key: Option<&str>,
		json_body: Option<Value>,
	) -> (StatusCode, Value) {
		let mut builder = Request::builder().method(method).uri(uri);

		if let Some(token) = auth_token {
			builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
		}
		if let Some(key) = idempotency_key {
			builder = builder.header("idempotency-key", key);
		}

		let body = match json_body {
			Some(payload) => {
				builder = builder.header(header::CONTENT_TYPE, "application/json");
				Body::from(payload.to_string())
			}
			None => Body::empty(),
		};

		let req = builder.body(body).expect("build request");
		let res = self.app.clone().oneshot(req).await.expect("execute request");

		let status = res.status();
		let bytes = res.into_body().collect().await.expect("read body").to_bytes();
		let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

		(status, body)
	}
}

#[derive(Clone)]
pub struct UserSession {
	pub user: User,
	pub token: String,
	pub harness: TestHarness,
}

impl UserSession {
	pub async fn create_account(&self, name: &str, atype: AccountType) -> (StatusCode, Value) {
		self
			.harness
			.request_raw(
				Method::POST,
				"/accounts",
				Some(&self.token),
				Some(&format!("acc-key-{}", Uuid::new_v4())),
				Some(json!({
          "name": name,
          "account_type": atype
        })),
			)
			.await
	}

	pub async fn get_account(&self, id: i32) -> (StatusCode, Value) {
		self
			.harness
			.request_raw(Method::GET, &format!("/accounts/{id}"), Some(&self.token), None, None)
			.await
	}

	pub async fn get_balance(&self, id: i32) -> Decimal {
		let (status, body) = self
			.harness
			.request_raw(Method::GET, &format!("/accounts/{id}/balance"), Some(&self.token), None, None)
			.await;
		assert_eq!(status, StatusCode::OK);
		body["balance"].as_str().unwrap().parse().expect("valid decimal string")
	}

	pub async fn transfer(
		&self,
		source_id: i32,
		dest_id: i32,
		amount: &str,
		description: &str,
		key: Option<&str>,
	) -> (StatusCode, Value) {
		let auto_key = format!("txn-{}", Uuid::new_v4());
		let id_key = key.unwrap_or(&auto_key);

		self
			.harness
			.request_raw(
				Method::POST,
				"/transactions",
				Some(&self.token),
				Some(id_key),
				Some(json!({
          "source_account_id": source_id,
          "destination_account_id": dest_id,
          "amount": amount,
          "description": description
        })),
			)
			.await
	}

	pub async fn reverse(
		&self,
		txn_id: i64,
		reason: &str,
		key: Option<&str>,
	) -> (StatusCode, Value) {
		let auto_key = format!("rev-{}", Uuid::new_v4());
		let id_key = key.unwrap_or(&auto_key);

		self
			.harness
			.request_raw(
				Method::POST,
				&format!("/transactions/{txn_id}/reverse"),
				Some(&self.token),
				Some(id_key),
				Some(json!({ "reason": reason })),
			)
			.await
	}

	pub async fn create_funded_pair(&self, asset_name: &str, equity_name: &str, amount: &str) -> (i32, i32) {
		let (_, asset) = self.create_account(asset_name, AccountType::Asset).await;
		let (_, equity) = self.create_account(equity_name, AccountType::Equity).await;
		let asset_id = asset["id"].as_i64().unwrap() as i32;
		let equity_id = equity["id"].as_i64().unwrap() as i32;

		if amount != "0" && amount != "0.00" {
			let (status, _) = self.transfer(equity_id, asset_id, amount, "Initial Funding", None).await;
			assert_eq!(status, StatusCode::CREATED);
		}

		(asset_id, equity_id)
	}
}