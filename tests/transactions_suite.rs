// tests/transactions_suite.rs
mod common;

use axum::http::StatusCode;
use common::TestHarness;
use rust_decimal::dec;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test]
async fn test_successful_transfer(pool: PgPool) {
  let harness = TestHarness::new(pool);
  let (cash, equity) = harness
    .create_funded_pair("Cash", "Equity", dec!(1000.00))
    .await;

  let payload = json!({
      "source_account_id": cash.id,
      "destination_account_id": equity.id,
      "amount": "250.00",
      "description": "Payout"
  });

  let (status, body) = harness.post_json("/transactions", payload).await;

  assert_eq!(status, StatusCode::CREATED);
  assert!(body["transaction_id"].is_i64());

  // Verify source deducted
  let (_, cash_bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(cash_bal["balance"], "750.00");

  // Verify destination incremented
  let (_, eq_bal) = harness
    .get(&format!("/accounts/{}/balance", equity.id))
    .await;
  assert_eq!(eq_bal["balance"], "-750.00");
}

#[sqlx::test]
async fn test_overdraft_prevention_rejection(pool: PgPool) {
  let harness = TestHarness::new(pool);
  let (cash, equity) = harness
    .create_funded_pair("Checking", "Equity", dec!(100.00))
    .await;

  let payload = json!({
      "source_account_id": cash.id,
      "destination_account_id": equity.id,
      "amount": "150.00", // ₹50 overdraft
      "description": "Exceeding balance"
  });

  let (status, _) = harness.post_json("/transactions", payload).await;
  assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

  // Verify state remained untouched
  let (_, cash_bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(cash_bal["balance"], "100.00");
}

#[sqlx::test]
async fn test_reject_self_transfer(pool: PgPool) {
  let harness = TestHarness::new(pool);
  let (cash, _) = harness
    .create_funded_pair("Checking", "Equity", dec!(100.00))
    .await;

  let payload = json!({
      "source_account_id": cash.id,
      "destination_account_id": cash.id, // Same account
      "amount": "20.00",
      "description": "Wash trade"
  });

  let (status, _) = harness.post_json("/transactions", payload).await;
  assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn test_reject_zero_or_negative_amount(pool: PgPool) {
  let harness = TestHarness::new(pool);
  let (cash, equity) = harness
    .create_funded_pair("Checking", "Equity", dec!(100.00))
    .await;

  let payload_zero = json!({
      "source_account_id": cash.id,
      "destination_account_id": equity.id,
      "amount": "0.00",
      "description": "Zero transfer"
  });

  let (status_zero, _) = harness.post_json("/transactions", payload_zero).await;
  assert_eq!(status_zero, StatusCode::BAD_REQUEST);

  let payload_neg = json!({
      "source_account_id": cash.id,
      "destination_account_id": equity.id,
      "amount": "-50.00",
      "description": "Negative transfer"
  });

  let (status_neg, _) = harness.post_json("/transactions", payload_neg).await;
  assert_eq!(status_neg, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn test_transfer_missing_idempotency_key_fails(pool: PgPool) {
  let harness = TestHarness::new(pool);
  let (cash, equity) = harness
    .create_funded_pair("Cash", "Equity", dec!(500.00))
    .await;

  let payload = json!({
      "source_account_id": cash.id,
      "destination_account_id": equity.id,
      "amount": "50.00",
      "description": "Missing header transfer"
  });

  // Send POST without idempotency-key
  let (status, body) = harness
    .post_without_idempotency_key("/transactions", payload)
    .await;

  assert_eq!(status, StatusCode::BAD_REQUEST);
  assert_eq!(body["error"], "idempotency-key not found in headers");
}

#[sqlx::test]
async fn test_reversal_missing_idempotency_key_fails(pool: PgPool) {
  let harness = TestHarness::new(pool);
  let (cash, equity) = harness
    .create_funded_pair("Cash", "Equity", dec!(500.00))
    .await;

  // Create a transaction first
  let payload = json!({
      "source_account_id": cash.id,
      "destination_account_id": equity.id,
      "amount": "100.00",
      "description": "Pre-reversal transfer"
  });

  let (_, body) = harness.post_json("/transactions", payload).await;
  let txn_id = body["transaction_id"].as_i64().unwrap();

  // Try reversing without header
  let (status, err_body) = harness
    .post_without_idempotency_key(
      &format!("/transactions/{txn_id}/reverse"),
      json!({ "reason": "Accidental charge" }),
    )
    .await;

  assert_eq!(status, StatusCode::BAD_REQUEST);
  assert_eq!(err_body["error"], "idempotency-key not found in headers");
}
