mod common;

use axum::http::StatusCode;
use common::TestHarness;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test]
async fn test_create_account_lifecycle(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

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
  assert_eq!(body["owner_id"], harness.default_user.id);
  let account_id = body["id"].as_i64().expect("Expected account id") as i32;

  // 2. Success: Fetch Account by ID
  let (get_status, get_body) = harness.get(&format!("/accounts/{account_id}")).await;

  assert_eq!(get_status, StatusCode::OK);
  assert_eq!(get_body["id"], account_id);
  assert_eq!(get_body["name"], "Checking Account");
  assert_eq!(get_body["owner_id"], harness.default_user.id);

  // 3. Success: Fetch Fresh Account Balance
  let (bal_status, bal_body) = harness
    .get(&format!("/accounts/{account_id}/balance"))
    .await;

  assert_eq!(bal_status, StatusCode::OK);
  assert_eq!(bal_body["balance"], "0");
}

#[sqlx::test]
async fn test_account_not_found(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  let (status, _) = harness.get("/accounts/999999").await;
  assert_eq!(status, StatusCode::NOT_FOUND);

  let (bal_status, _) = harness.get("/accounts/999999/balance").await;
  assert_eq!(bal_status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn test_create_account_invalid_payload(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

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

  assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn test_accounts_endpoints_require_auth(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  // Unauthenticated creation attempt
  let (post_status, _) = harness
    .post_unauthenticated(
      "/accounts",
      json!({
        "name": "Unauthorized Account",
        "account_type": "Asset"
      }),
    )
    .await;
  assert_eq!(post_status, StatusCode::UNAUTHORIZED);

  // Unauthenticated fetch attempt
  let (get_status, _) = harness.get_unauthenticated("/accounts/1").await;
  assert_eq!(get_status, StatusCode::UNAUTHORIZED);

  // Unauthenticated balance attempt
  let (bal_status, _) = harness.get_unauthenticated("/accounts/1/balance").await;
  assert_eq!(bal_status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn test_cannot_access_other_users_account(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  // 1. User A creates an account via harness
  let (status, body) = harness
    .post_json(
      "/accounts",
      json!({
        "name": "User A Private Vault",
        "account_type": "Asset"
      }),
    )
    .await;
  assert_eq!(status, StatusCode::CREATED);
  let user_a_account_id = body["id"].as_i64().unwrap() as i32;

  // 2. Create User B in the database and issue a token for User B
  let (_, user_b_token) = harness.create_user("user_b", "pass_b_9999").await;

  // 3. User B attempts to access User A's account
  // If your policy hides foreign resources, expect 404; if it explicitly forbids, expect 403.
  let (get_status, _) = harness
    .request(
      axum::http::Method::GET,
      &format!("/accounts/{user_a_account_id}"),
      Some(&user_b_token),
      None,
      None,
    )
    .await;
  assert_eq!(get_status, StatusCode::FORBIDDEN);

  // 4. User B attempts to read User A's balance
  let (bal_status, _) = harness
    .request(
      axum::http::Method::GET,
      &format!("/accounts/{user_a_account_id}/balance"),
      Some(&user_b_token),
      None,
      None,
    )
    .await;
  assert_eq!(bal_status, StatusCode::FORBIDDEN);
}

#[sqlx::test]
async fn test_create_account_empty_name(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  let (status, _) = harness
    .post_json(
      "/accounts",
      json!({
        "name": "   ",
        "account_type": "Asset"
      }),
    )
    .await;

  assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn test_duplicate_account_name_same_user_rejected(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  // 1. Create initial account
  let (status, _) = harness
    .post_json(
      "/accounts",
      json!({
        "name": "Emergency Fund",
        "account_type": "Asset"
      }),
    )
    .await;
  assert_eq!(status, StatusCode::CREATED);

  // 2. Attempt duplicate creation under same user -> 409 Conflict
  let (dup_status, dup_body) = harness
    .post_json(
      "/accounts",
      json!({
        "name": "Emergency Fund",
        "account_type": "Asset"
      }),
    )
    .await;
  assert_eq!(dup_status, StatusCode::CONFLICT);
  assert!(
    dup_body["error"]
      .as_str()
      .unwrap()
      .contains("already exists")
  );
}

#[sqlx::test]
async fn test_duplicate_account_name_different_users_allowed(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  // 1. Default user creates "Savings"
  let (status_a, body_a) = harness
    .post_json(
      "/accounts",
      json!({
        "name": "Savings",
        "account_type": "Asset"
      }),
    )
    .await;
  assert_eq!(status_a, StatusCode::CREATED);

  // 2. Create User B
  let (user_b, user_b_token) = harness.create_user("user_b_savings", "pass_b_9999").await;

  // 3. User B creates "Savings" -> should succeed
  let (status_b, body_b) = harness
    .request(
      axum::http::Method::POST,
      "/accounts",
      Some(&user_b_token),
      Some("auto-key-user-b"),
      Some(json!({
        "name": "Savings",
        "account_type": "Asset"
      })),
    )
    .await;

  assert_eq!(status_b, StatusCode::CREATED);
  assert_ne!(body_a["id"], body_b["id"]);
  assert_eq!(body_b["owner_id"], user_b.id);
}

#[sqlx::test]
async fn test_get_all_accounts_filters_by_owner(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  // User A creates two accounts
  harness
    .post_json(
      "/accounts",
      json!({ "name": "Checking", "account_type": "Asset" }),
    )
    .await;
  harness
    .post_json(
      "/accounts",
      json!({ "name": "Vault", "account_type": "Asset" }),
    )
    .await;

  // User B creates one account
  let (_, user_b_token) = harness.create_user("user_b_all", "pass_b_all").await;

  harness
    .request(
      axum::http::Method::POST,
      "/accounts",
      Some(&user_b_token),
      Some("user-b-key"),
      Some(json!({ "name": "User B Account", "account_type": "Asset" })),
    )
    .await;

  // Fetch all accounts as User A -> should only see 2 accounts
  let (status, body) = harness.get("/accounts").await;
  assert_eq!(status, StatusCode::OK);
  let accounts = body.as_array().expect("Expected array of accounts");
  assert_eq!(accounts.len(), 2);
  assert!(
    accounts
      .iter()
      .all(|a| a["owner_id"] == harness.default_user.id)
  );
}
