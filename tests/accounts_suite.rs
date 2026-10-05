mod common;

use axum::http::StatusCode;
use common::TestHarness;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test]
async fn test_create_account_lifecycle(pool: PgPool) {
  let harness = TestHarness::new(pool);

  // 1. Success: Create Asset Account
  let (status, body) = harness
    .post_json(
      "/accounts",
      json!({
          "name": "Checking Account",
          "account_type": "Asset"
      }),
    )
    .await;

  assert_eq!(status, StatusCode::CREATED);
  assert_eq!(body["name"], "Checking Account");
  assert_eq!(body["account_type"], "Asset");
  let account_id = body["id"].as_i64().expect("Expected account id") as i32;

  // 2. Success: Fetch Account by ID
  let (get_status, get_body) = harness.get(&format!("/accounts/{account_id}")).await;

  assert_eq!(get_status, StatusCode::OK);
  assert_eq!(get_body["id"], account_id);
  assert_eq!(get_body["name"], "Checking Account");

  // 3. Success: Fetch Fresh Account Balance
  let (bal_status, bal_body) = harness
    .get(&format!("/accounts/{account_id}/balance"))
    .await;

  assert_eq!(bal_status, StatusCode::OK);
  assert_eq!(bal_body["balance"], "0");
}

#[sqlx::test]
async fn test_account_not_found(pool: PgPool) {
  let harness = TestHarness::new(pool);

  let (status, _) = harness.get("/accounts/999999").await;
  assert_eq!(status, StatusCode::NOT_FOUND);

  let (bal_status, _) = harness.get("/accounts/999999/balance").await;
  assert_eq!(bal_status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn test_create_account_invalid_payload(pool: PgPool) {
  let harness = TestHarness::new(pool);

  // Invalid account type enum variant
  let (status, _) = harness
    .post_json(
      "/accounts",
      json!({
          "name": "Test",
          "account_type": "NonExistentType"
      }),
    )
    .await;

  assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
