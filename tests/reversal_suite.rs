mod common;

use axum::http::StatusCode;
use common::TestHarness;
use rust_decimal::dec;
use serde_json::json;
use sqlx::PgPool;
use txn::db::rows::AccountType;

#[sqlx::test]
async fn test_transaction_reversal_lifecycle(pool: PgPool) {
  let harness = TestHarness::new(pool).await;
  let (cash, equity) = harness
    .create_funded_pair("Cash", "Equity", dec!(1000.00))
    .await;

  // 1. Spend ₹400
  let payload = json!({
    "source_account_id": cash.id,
    "destination_account_id": equity.id,
    "amount": "400.00",
    "description": "Erroneous transfer"
  });

  let (_, body) = harness.post_json("/transactions", payload).await;
  let original_txn_id = body["transaction_id"].as_i64().unwrap() as i32;

  // Verify balance after spend: ₹600.00
  let (_, bal_before) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(bal_before["balance"], "600.00");

  // 2. Reverse the transaction
  let (rev_status, rev_body) = harness
    .post_json(
      &format!("/transactions/{original_txn_id}/reversal"),
      json!({ "reason": "Customer cancellation" }),
    )
    .await;

  assert_eq!(rev_status, StatusCode::CREATED);
  let reversal_txn_id = rev_body["transaction_id"].as_i64().unwrap() as i32;

  // Verify balance is restored: ₹1000.00
  let (_, bal_after) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(bal_after["balance"], "1000.00");

  // 3. Double-Reversal Guard: Attempt to reverse the same transaction with a NEW idempotency key
  let (dup_status, dup_body) = harness
    .post_json(
      &format!("/transactions/{original_txn_id}/reversal"),
      json!({ "reason": "Second attempt" }),
    )
    .await;
  assert_eq!(dup_status, StatusCode::UNPROCESSABLE_ENTITY);
  assert!(
    dup_body["error"]
      .as_str()
      .unwrap()
      .to_lowercase()
      .contains("already been reversed")
  );

  // 4. Reversal-of-Reversal Guard: Attempt to reverse the reversal itself
  let (circ_status, _) = harness
    .post_json(
      &format!("/transactions/{reversal_txn_id}/reversal"),
      json!({ "reason": "Reverse the reversal" }),
    )
    .await;
  assert_eq!(circ_status, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn test_reversal_of_nonexistent_transaction_returns_not_found(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  let (status, _) = harness
    .post_json(
      "/transactions/999999/reversal",
      json!({ "reason": "Nonexistent target" }),
    )
    .await;

  assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn test_reversal_rejected_when_recipient_lacks_funds(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  // 1. Account A (Cash) starts with ₹500, Account B (Vendor) starts with ₹0
  let (cash, _) = harness
    .create_funded_pair("Cash", "Equity", dec!(500.00))
    .await;
  let vendor = harness.create_account("Vendor", AccountType::Asset).await;

  // 2. Transfer ₹300 from Cash to Vendor
  let (_, tx_body) = harness
    .post_json(
      "/transactions",
      json!({
        "source_account_id": cash.id,
        "destination_account_id": vendor.id,
        "amount": "300.00",
        "description": "Payment to vendor"
      }),
    )
    .await;
  let original_txn_id = tx_body["transaction_id"].as_i64().unwrap() as i32;

  // 3. Vendor spends ₹250 to another account (Equity), leaving only ₹50
  let equity2 = harness
    .create_account("Vendor Equity", AccountType::Equity)
    .await;
  let (spend_status, _) = harness
    .post_json(
      "/transactions",
      json!({
        "source_account_id": vendor.id,
        "destination_account_id": equity2.id,
        "amount": "250.00",
        "description": "Vendor spending their income"
      }),
    )
    .await;
  assert_eq!(spend_status, StatusCode::CREATED);

  // 4. Attempt to reverse original ₹300 transfer:
  // Reversal requires pulling ₹300 back from Vendor, but Vendor only has ₹50.
  let (rev_status, rev_body) = harness
    .post_json(
      &format!("/transactions/{original_txn_id}/reversal"),
      json!({ "reason": "Customer chargeback" }),
    )
    .await;

  // Invariant: Must fail with insufficient funds on the recipient account
  assert_eq!(rev_status, StatusCode::UNPROCESSABLE_ENTITY);
  assert!(
    rev_body["error"]
      .as_str()
      .unwrap()
      .to_lowercase()
      .contains("insufficient")
  );

  // 5. Verify Cash balance remained at ₹200 (500 - 300) without unearned refund
  let (_, cash_bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(cash_bal["balance"], "200.00");
}

#[sqlx::test]
async fn test_reversal_validates_reason_payload(pool: PgPool) {
  let harness = TestHarness::new(pool).await;
  let (cash, equity) = harness
    .create_funded_pair("Cash", "Equity", dec!(100.00))
    .await;

  let (_, tx_body) = harness
    .post_json(
      "/transactions",
      json!({
        "source_account_id": cash.id,
        "destination_account_id": equity.id,
        "amount": "20.00",
        "description": "Legit transfer"
      }),
    )
    .await;
  let txn_id = tx_body["transaction_id"].as_i64().unwrap();

  // Empty reason
  let (status_empty, _) = harness
    .post_json(
      &format!("/transactions/{txn_id}/reversal"),
      json!({ "reason": "" }),
    )
    .await;
  assert_eq!(status_empty, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn test_cross_tenant_reversal_forbidden_for_outsider(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  // 1. User A (default user) creates an internal transfer
  let (cash, equity) = harness
    .create_funded_pair("User A Cash", "User A Equity", dec!(500.00))
    .await;

  let (_, tx_body) = harness
    .post_json(
      "/transactions",
      json!({
        "source_account_id": cash.id,
        "destination_account_id": equity.id,
        "amount": "100.00",
        "description": "Internal User A reallocation"
      }),
    )
    .await;
  let txn_id = tx_body["transaction_id"].as_i64().unwrap();

  // 2. Create User B (outsider tenant)
  let (_, user_b_token) = harness.create_user("outsider_b", "pass_b_secure").await;

  // 3. User B attempts to reverse User A's transaction
  let (rev_status, rev_body) = harness
    .request(
      axum::http::Method::POST,
      &format!("/transactions/{txn_id}/reversal"),
      Some(&user_b_token),
      Some("outsider-rev-key"),
      Some(json!({ "reason": "Malicious foreign reversal" })),
    )
    .await;

  assert_eq!(rev_status, StatusCode::FORBIDDEN);
  assert!(
    rev_body["error"]
      .as_str()
      .unwrap()
      .contains("not authorized to reverse this transaction")
  );
}

#[sqlx::test]
async fn test_concurrent_double_reversal_returns_422_not_500(pool: PgPool) {
  let harness = std::sync::Arc::new(TestHarness::new(pool).await);
  let (cash, equity) = harness
    .create_funded_pair("CashConc", "EquityConc", dec!(1000.00))
    .await;

  // 1. Create target transaction
  let (_, body) = harness
    .post_json(
      "/transactions",
      json!({
        "source_account_id": cash.id,
        "destination_account_id": equity.id,
        "amount": "200.00",
        "description": "Original transfer to reverse concurrently"
      }),
    )
    .await;
  let target_txn_id = body["transaction_id"].as_i64().unwrap();

  // 2. Fire 2 concurrent reversals with DIFFERENT idempotency keys
  let h1 = std::sync::Arc::clone(&harness);
  let h2 = std::sync::Arc::clone(&harness);

  let task1 = tokio::spawn(async move {
    h1.post_with_idempotency(
      &format!("/transactions/{target_txn_id}/reversal"),
      "rev-race-key-1",
      json!({ "reason": "Concurrent reason 1" }),
    )
    .await
  });

  let task2 = tokio::spawn(async move {
    h2.post_with_idempotency(
      &format!("/transactions/{target_txn_id}/reversal"),
      "rev-race-key-2",
      json!({ "reason": "Concurrent reason 2" }),
    )
    .await
  });

  let (res1, res2) = tokio::join!(task1, task2);
  let (status1, _) = res1.expect("task 1 panicked");
  let (status2, _) = res2.expect("task 2 panicked");

  let statuses = vec![status1, status2];

  // Exactly one must succeed (201 CREATED) and one must be rejected (422 UNPROCESSABLE_ENTITY)
  assert!(
    statuses.contains(&StatusCode::CREATED),
    "One request must succeed with 201 Created: {statuses:?}"
  );
  assert!(
    statuses.contains(&StatusCode::UNPROCESSABLE_ENTITY),
    "The losing race must receive 422 Unprocessable Entity (not 500): {statuses:?}"
  );

  // Assert no 500 error occurred
  assert_ne!(status1, StatusCode::INTERNAL_SERVER_ERROR);
  assert_ne!(status2, StatusCode::INTERNAL_SERVER_ERROR);

  // Balance must be restored to 1000.00, not double-reversed to 1200.00
  let (_, bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(bal["balance"], "1000.00");
}
