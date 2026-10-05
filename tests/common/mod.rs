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
use tower::ServiceExt;
use uuid::Uuid;

use txn::api::AppRouter;
use txn::api::AppState;
use txn::db::Repo;
use txn::db::schemas::{Account, AccountType};
use txn::models::draft::{PostingDraft, TransactionDraft};

pub struct TestHarness {
  pub repo: Repo,
  app: Router,
}

impl TestHarness {
  pub fn new(pool: PgPool) -> Self {
    let repo = Repo::from_pool(pool);
    let state = AppState::new(repo.clone());
    let app = AppRouter::with_state(state);
    Self { repo, app }
  }

  // --- Direct-to-DB Fixture Helpers ---

  pub async fn create_account(&self, name: &str, atype: AccountType) -> Account {
    self
      .repo
      .create_account(name, atype)
      .await
      .expect("Failed to seed account")
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

  /// Default POST: auto-generates a unique Idempotency-Key for endpoints that require it
  pub async fn post_json(&self, uri: &str, body: Value) -> (StatusCode, Value) {
    let auto_key = format!("test-{}", Uuid::new_v4());
    self
      .request(Method::POST, uri, Some(&auto_key), Some(body))
      .await
  }

  /// Replay POST: uses the exact idempotency key supplied
  pub async fn post_with_idempotency(
    &self,
    uri: &str,
    idempotency_key: &str,
    body: Value,
  ) -> (StatusCode, Value) {
    self
      .request(Method::POST, uri, Some(idempotency_key), Some(body))
      .await
  }

  /// Header-omitted POST: explicitly skips the Idempotency-Key header
  pub async fn post_without_idempotency_key(&self, uri: &str, body: Value) -> (StatusCode, Value) {
    self.request(Method::POST, uri, None, Some(body)).await
  }

  pub async fn get(&self, uri: &str) -> (StatusCode, Value) {
    self.request(Method::GET, uri, None, None).await
  }

  async fn request(
    &self,
    method: Method,
    uri: &str,
    idempotency_key: Option<&str>,
    json_body: Option<Value>,
  ) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);

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
