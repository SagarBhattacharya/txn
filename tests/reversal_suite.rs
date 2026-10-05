mod common;

use axum::http::StatusCode;
use common::TestHarness;
use rust_decimal::dec;
use serde_json::json;
use sqlx::PgPool;
use txn::db::schemas::AccountType;

#[sqlx::test]
async fn test_transaction_reversal_lifecycle(pool: PgPool) {
  let harness = TestHarness::new(pool);
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
      &format!("/transactions/{original_txn_id}/reverse"),
      json!({ "reason": "Customer cancellation" }),
    )
    .await;

  assert_eq!(rev_status, StatusCode::CREATED);
  let reversal_txn_id = rev_body["transaction_id"].as_i64().unwrap() as i32;

  // Verify balance is restored: ₹1000.00
  let (_, bal_after) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(bal_after["balance"], "1000.00");

  // 3. Double-Reversal Guard: Attempt to reverse the same transaction again
  let (dup_status, _) = harness
    .post_json(
      &format!("/transactions/{original_txn_id}/reverse"),
      json!({ "reason": "Second attempt" }),
    )
    .await;
  assert_eq!(dup_status, StatusCode::UNPROCESSABLE_ENTITY);

  // 4. Reversal-of-Reversal Guard: Attempt to reverse the reversal itself
  let (circ_status, _) = harness
    .post_json(
      &format!("/transactions/{reversal_txn_id}/reverse"),
      json!({ "reason": "Reverse the reversal" }),
    )
    .await;
  assert_eq!(circ_status, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn test_reversal_of_nonexistent_transaction_returns_not_found(pool: PgPool) {
  let harness = TestHarness::new(pool);

  let (status, _) = harness
    .post_json(
      "/transactions/999999/reverse",
      json!({ "reason": "Nonexistent target" }),
    )
    .await;

  assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn test_reversal_rejected_when_recipient_lacks_funds(pool: PgPool) {
  let harness = TestHarness::new(pool);

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
  let (rev_status, _) = harness
    .post_json(
      &format!("/transactions/{original_txn_id}/reverse"),
      json!({ "reason": "Customer chargeback" }),
    )
    .await;

  // Invariant: Must fail with insufficient funds on the recipient account
  assert_eq!(rev_status, StatusCode::UNPROCESSABLE_ENTITY);

  // 5. Verify Cash balance remained at ₹200 (500 - 300) without unearned refund
  let (_, cash_bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(cash_bal["balance"], "200.00");
}
