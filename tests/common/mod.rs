#![allow(unused)]
use axum::{
  body::Body,
  http::{header, Method, Request, StatusCode},
  Router,
};
use http_body_util::BodyExt;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use txn::api::{AppRouter, AppState};
use txn::auth::{create_jwt, hash_password};
use txn::db::schemas::{Account, AccountType, User};
use txn::db::Repo;
use txn::models::draft::{PostingDraft, TransactionDraft};

pub struct TestHarness {
  pub repo: Repo,
  pub default_user: User,
  pub auth_token: String,
  pub jwt_secret: String,
  app: Router,
}

impl TestHarness {
  pub async fn new(pool: PgPool) -> Self {
    let test_secret = "rrNeXDdWY3ZahvdsLcd85usVVO34pvGuDfFI1i5tbOX";

    let repo = Repo::from_pool(pool);
    let state = AppState::new(repo.clone(), test_secret.into());
    let app = AppRouter::with_state(state.clone());

    // 1. Seed a default test user directly into DB
    let username = format!("user_{}", &Uuid::new_v4().to_string()[..8]);
    let password_hash = hash_password("test_pass_1234")
      .expect("Failed to hash test pass");

    let default_user = repo
      .create_user(&username, &password_hash)
      .await
      .expect("Failed to seed default test user")
      .expect("default user should not exist");

    // 2. Pre-generate a valid Bearer token for default user
    let auth_token = create_jwt(
      default_user.id,
      &default_user.username,
      state.jwt_secret.as_bytes(),
    ).expect("Failed to create test JWT");

    Self {
      repo,
      default_user,
      auth_token,
      app,
      jwt_secret: test_secret.into(),
    }
  }

  // --- Direct-to-DB Fixture Helpers ---

  pub async fn create_account(&self, name: &str, atype: AccountType) -> Option<Account> {
    self.create_account_for_user(self.default_user.id, name, atype).await
  }

  pub async fn create_account_for_user(
    &self,
    owner_id: i32,
    name: &str,
    atype: AccountType,
  ) -> Option<Account> {
    self
      .repo
      .create_account(owner_id, name, atype)
      .await
      .expect("Failed to seed account")
  }

  pub async fn create_funded_pair(
    &self,
    asset_name: &str,
    equity_name: &str,
    balance: Decimal,
  ) -> (Account, Account) {
    let asset = self.create_account(asset_name, AccountType::Asset)
      .await.expect("asset should not exist");

    let equity = self.create_account(equity_name, AccountType::Equity).await
      .expect("equity should not exist");

    if !balance.is_zero() {
      let seed_key = format!("seed-{}", Uuid::new_v4());
      let draft = TransactionDraft::new(
        "Initial Seed Funding".to_string(),
        seed_key,
        vec![
          PostingDraft::new(asset.id, balance).unwrap(),
          PostingDraft::new(equity.id, -balance).unwrap(),
        ],
      )
        .expect("Failed to construct seed draft");

      self
        .repo
        .record_transaction(draft)
        .await
        .expect("Failed to execute seed transaction");
    }

    (asset, equity)
  }

  // --- HTTP Request Helpers ---

  /// Default POST: injects Bearer token and generates a unique Idempotency-Key
  pub async fn post_json(&self, uri: &str, body: Value) -> (StatusCode, Value) {
    let auto_key = format!("test-{}", Uuid::new_v4());
    self
      .request(
        Method::POST,
        uri,
        Some(&self.auth_token),
        Some(&auto_key),
        Some(body),
      )
      .await
  }

  /// Replay POST: injects Bearer token with specific idempotency key
  pub async fn post_with_idempotency(
    &self,
    uri: &str,
    idempotency_key: &str,
    body: Value,
  ) -> (StatusCode, Value) {
    self
      .request(
        Method::POST,
        uri,
        Some(&self.auth_token),
        Some(idempotency_key),
        Some(body),
      )
      .await
  }

  /// Header-omitted POST: injects Bearer token but skips Idempotency-Key
  pub async fn post_without_idempotency_key(&self, uri: &str, body: Value) -> (StatusCode, Value) {
    self
      .request(Method::POST, uri, Some(&self.auth_token), None, Some(body))
      .await
  }

  /// Unauthenticated POST: explicitly omits Authorization header
  pub async fn post_unauthenticated(&self, uri: &str, body: Value) -> (StatusCode, Value) {
    let auto_key = format!("test-{}", Uuid::new_v4());
    self
      .request(Method::POST, uri, None, Some(&auto_key), Some(body))
      .await
  }

  /// Authenticated GET
  pub async fn get(&self, uri: &str) -> (StatusCode, Value) {
    self
      .request(Method::GET, uri, Some(&self.auth_token), None, None)
      .await
  }

  /// Unauthenticated GET
  pub async fn get_unauthenticated(&self, uri: &str) -> (StatusCode, Value) {
    self.request(Method::GET, uri, None, None, None).await
  }

  pub async fn request(
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

    let body = if let Some(payload) = json_body {
      builder = builder.header(header::CONTENT_TYPE, "application/json");
      Body::from(payload.to_string())
    } else {
      Body::empty()
    };

    let req = builder.body(body).expect("Failed to construct request");
    let res = self
      .app
      .clone()
      .oneshot(req)
      .await
      .expect("App request failed");

    let status = res.status();
    let bytes = res
      .into_body()
      .collect()
      .await
      .expect("Failed to read body")
      .to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    (status, body)
  }
}