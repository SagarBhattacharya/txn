#![allow(unused)]

use axum::{
  Router,
  body::Body,
  http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use txn::app::auth::{JwtKeys, hash_password};
use txn::app::{AppState, router};
use txn::core::errors::Error;
use txn::core::ledger;
use txn::core::types::{Amount, IdempotencyKey, Note, TransferCmd};
use txn::db::queries::Query;
use txn::db::rows::{Account, AccountType, User};
use uuid::Uuid;

pub struct TestHarness {
  pub pool: PgPool,
  pub default_user: User,
  pub auth_token: String,
  pub jwt: Arc<JwtKeys>,
  app: Router,
}

impl TestHarness {
  pub async fn new(pool: PgPool) -> Self {
    let test_secret = "rrNeXDdWY3ZahvdsLcd85usVVO34pvGuDfFI1i5tbOX";
    let jwt = Arc::new(JwtKeys::new(test_secret));

    let state = AppState::new(pool.clone(), jwt.clone());
    let app = router(state);

    // 1. Seed a default test user directly into DB
    let username = format!("user_{}", &Uuid::new_v4().to_string()[..8]);
    let password_hash = hash_password("test_pass_1234").expect("Failed to hash test pass");

    let default_user = Query::create_user(&pool, &username, &password_hash)
      .await
      .expect("Failed to seed default user");

    // 2. Pre-generate a valid Bearer token for default user
    let auth_token = jwt
      .issue(default_user.id)
      .expect("Failed to create test JWT");

    Self {
      pool,
      default_user,
      auth_token,
      app,
      jwt,
    }
  }

  // --- Direct-to-DB Fixture Helpers ---

  pub async fn create_account(&self, name: &str, atype: AccountType) -> Account {
    self
      .create_account_for_user(self.default_user.id, name, atype)
      .await
  }

  pub async fn create_user(&self, username: &str, password: &str) -> (User, String) {
    let pass_hash = hash_password(password).unwrap();
    let user = Query::create_user(&self.pool, &username, &pass_hash)
      .await
      .expect("Failed to create user");

    let auth_token = self.jwt.issue(user.id).expect("Failed to create JWT");

    (user, auth_token)
  }

  pub async fn create_account_for_user(
    &self,
    owner_id: i32,
    name: &str,
    atype: AccountType,
  ) -> Account {
    Query::create_account(&self.pool, owner_id, name, atype)
      .await
      .expect("Failed to seed account")
  }

  /// Probe duplicate creation: returns None on Conflict (per REFACTOR.md section 4)
  pub async fn create_account_probe(
    &self,
    owner_id: i32,
    name: &str,
    atype: AccountType,
  ) -> Option<Account> {
    match Query::create_account(&self.pool, owner_id, name, atype)
      .await
      .map_err(Error::from)
    {
      Ok(acc) => Some(acc),
      Err(Error::Conflict(_)) => None,
      Err(err) => panic!("Unexpected database error seeding account: {err}"),
    }
  }

  pub async fn create_funded_pair(
    &self,
    asset_name: &str,
    equity_name: &str,
    balance: Decimal,
  ) -> (Account, Account) {
    let asset = self.create_account(asset_name, AccountType::Asset).await;
    let equity = self.create_account(equity_name, AccountType::Equity).await;

    if !balance.is_zero() {
      let seed_key =
        IdempotencyKey::try_from(format!("seed-{}", Uuid::new_v4())).expect("valid seed key");
      let amount = Amount::try_from(balance).expect("valid balance amount");
      let description =
        Note::try_from("Initial Seed Funding".to_string()).expect("valid description");

      let cmd = TransferCmd {
        source_account_id: equity.id,
        destination_account_id: asset.id,
        amount,
        description,
      };

      ledger::transfer(&self.pool, self.default_user.id, &seed_key, &cmd)
        .await
        .expect("Failed to seed funded pair via ledger pipeline");
    }

    (asset, equity)
  }

  // --- HTTP Request Helpers ---

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

  pub async fn post_without_idempotency_key(&self, uri: &str, body: Value) -> (StatusCode, Value) {
    self
      .request(Method::POST, uri, Some(&self.auth_token), None, Some(body))
      .await
  }

  pub async fn post_unauthenticated(&self, uri: &str, body: Value) -> (StatusCode, Value) {
    let auto_key = format!("test-{}", Uuid::new_v4());
    self
      .request(Method::POST, uri, None, Some(&auto_key), Some(body))
      .await
  }

  pub async fn get(&self, uri: &str) -> (StatusCode, Value) {
    self
      .request(Method::GET, uri, Some(&self.auth_token), None, None)
      .await
  }

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
