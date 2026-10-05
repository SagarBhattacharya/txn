mod common;

use axum::http::StatusCode;
use common::TestHarness;
use rust_decimal::dec;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test]
async fn test_idempotent_replay_returns_cached_transaction(pool: PgPool) {
  let harness = TestHarness::new(pool);
  let (cash, equity) = harness
    .create_funded_pair("Cash", "Equity", dec!(500.00))
    .await;

  let payload = json!({
      "source_account_id": cash.id,
      "destination_account_id": equity.id,
      "amount": "100.00",
      "description": "Bill Payment"
  });

  let idempotency_key = "idemp-key-test-8899";

  // First attempt: succeeds and creates record
  let (status1, body1) = harness
    .post_with_idempotency("/transactions", idempotency_key, payload.clone())
    .await;

  assert_eq!(status1, StatusCode::CREATED);
  let txn_id_1 = body1["transaction_id"].as_i64().unwrap();

  // Second attempt: exact same payload and key (simulating network timeout retry)
  let (status2, body2) = harness
    .post_with_idempotency("/transactions", idempotency_key, payload)
    .await;

  assert!(status2 == StatusCode::OK || status2 == StatusCode::CREATED);
  let txn_id_2 = body2["transaction_id"].as_i64().unwrap();

  // Verify IDs match exactly
  assert_eq!(txn_id_1, txn_id_2);

  // Verify account was debited once (500 - 100 = 400)
  let (_, cash_bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(cash_bal["balance"], "400.00");
}

#[sqlx::test]
async fn test_concurrent_identical_requests_execute_exactly_once(pool: PgPool) {
  let harness = std::sync::Arc::new(TestHarness::new(pool));
  let (cash, equity) = harness
    .create_funded_pair("ConcurrentCash", "ConcurrentEquity", dec!(1000.00))
    .await;

  let payload = json!({
      "source_account_id": cash.id,
      "destination_account_id": equity.id,
      "amount": "200.00",
      "description": "Concurrent retry hammer"
  });

  let shared_idempotency_key = "idemp-race-token-uuid-999";
  let num_tasks = 10;
  let mut handles = Vec::new();

  // Fire 10 parallel requests with the identical idempotency key
  for _ in 0..num_tasks {
    let h = std::sync::Arc::clone(&harness);
    let p = payload.clone();
    handles.push(tokio::spawn(async move {
      h.post_with_idempotency("/transactions", shared_idempotency_key, p)
        .await
    }));
  }

  let results = futures::future::join_all(handles).await;

  let mut returned_ids = Vec::new();
  for res in results {
    let (status, body) = res.expect("Task panicked");
    assert!(
      status == StatusCode::CREATED || status == StatusCode::OK,
      "Unexpected status: {status}"
    );
    let txn_id = body["transaction_id"]
      .as_i64()
      .expect("Missing transaction_id");
    returned_ids.push(txn_id);
  }

  // Invariant 1: All 10 tasks returned the exact same transaction ID
  let first_id = returned_ids[0];
  assert!(
    returned_ids.iter().all(|&id| id == first_id),
    "Mismatch in transaction IDs under concurrency: {returned_ids:?}"
  );

  // Invariant 2: Balance was deducted exactly once (1000 - 200 = 800)
  let (_, cash_bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(cash_bal["balance"], "800.00");
}
