mod common;

use axum::http::StatusCode;
use common::TestHarness;
use rust_decimal::dec;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use txn::db::rows::AccountType;

#[sqlx::test]
async fn test_idempotent_replay_returns_cached_transaction(pool: PgPool) {
  let harness = TestHarness::new(pool).await;
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

  assert_eq!(status2, StatusCode::CREATED);
  let txn_id_2 = body2["transaction_id"].as_i64().unwrap();

  // Verify IDs match exactly
  assert_eq!(txn_id_1, txn_id_2);

  // Verify account was debited once (500 - 100 = 400)
  let (_, cash_bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(cash_bal["balance"], "400.00");
}

#[sqlx::test]
async fn test_concurrent_identical_requests_execute_exactly_once(pool: PgPool) {
  let harness = Arc::new(TestHarness::new(pool).await);
  let (cash, equity) = harness
    .create_funded_pair("ConcurrentCash", "ConcurrentEquity", dec!(200.00))
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
    let h = Arc::clone(&harness);
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

  // Invariant 2: Balance was deducted exactly once (200- 200 = 0)
  let (_, cash_bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(cash_bal["balance"], "0");
}

#[sqlx::test]
async fn test_reversal_idempotent_replay_returns_cached_transaction(pool: PgPool) {
  let harness = TestHarness::new(pool).await;
  let (cash, equity) = harness
    .create_funded_pair("RevCash", "RevEquity", dec!(500.00))
    .await;

  // 1. Initial transaction
  let (_, body) = harness
    .post_json(
      "/transactions",
      json!({
        "source_account_id": cash.id,
        "destination_account_id": equity.id,
        "amount": "150.00",
        "description": "Original purchase"
      }),
    )
    .await;
  let orig_txn_id = body["transaction_id"].as_i64().unwrap();

  let rev_idempotency_key = "rev-idemp-key-5566";
  let rev_payload = json!({ "reason": "Accidental double-click" });

  // 2. First reversal attempt
  let (rev_status1, rev_body1) = harness
    .post_with_idempotency(
      &format!("/transactions/{orig_txn_id}/reversal"),
      rev_idempotency_key,
      rev_payload.clone(),
    )
    .await;

  assert_eq!(rev_status1, StatusCode::CREATED);
  let rev_txn_id_1 = rev_body1["transaction_id"].as_i64().unwrap();

  // 3. Second reversal attempt: same key and payload
  let (rev_status2, rev_body2) = harness
    .post_with_idempotency(
      &format!("/transactions/{orig_txn_id}/reversal"),
      rev_idempotency_key,
      rev_payload,
    )
    .await;

  assert_eq!(rev_status2, StatusCode::CREATED);
  let rev_txn_id_2 = rev_body2["transaction_id"].as_i64().unwrap();

  // Invariant 1: Exactly the same reversal ID returned
  assert_eq!(rev_txn_id_1, rev_txn_id_2);

  // Invariant 2: Balance restored to 500.00, not double-reversed to 650.00
  let (_, cash_bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(cash_bal["balance"], "500.00");
}

#[sqlx::test]
async fn test_reusing_key_with_different_payload_fails(pool: PgPool) {
  let harness = TestHarness::new(pool).await;
  let (cash, equity) = harness
    .create_funded_pair("Cash", "Equity", dec!(500.00))
    .await;

  let key = "reused-key-diff-payload";

  // 1. Initial request: ₹50
  let (status1, _) = harness
    .post_with_idempotency(
      "/transactions",
      key,
      json!({
        "source_account_id": cash.id,
        "destination_account_id": equity.id,
        "amount": "50.00",
        "description": "Payment 1"
      }),
    )
    .await;
  assert_eq!(status1, StatusCode::CREATED);

  // 2. Replay with SAME key, but DIFFERENT amount: ₹100
  let (status2, body2) = harness
    .post_with_idempotency(
      "/transactions",
      key,
      json!({
        "source_account_id": cash.id,
        "destination_account_id": equity.id,
        "amount": "100.00",
        "description": "Payment 1"
      }),
    )
    .await;

  assert_eq!(status2, StatusCode::UNPROCESSABLE_ENTITY);
  assert!(body2["error"].as_str().unwrap().contains("mismatched"));

  // Verify only ₹50 was deducted
  let (_, bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(bal["balance"], "450.00");
}

#[sqlx::test]
async fn test_different_users_can_use_same_idempotency_key(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  // User A transaction with key "core-key"
  let (cash_a, eq_a) = harness
    .create_funded_pair("Cash A", "Equity A", dec!(300.00))
    .await;

  let (status_a, body_a) = harness
    .post_with_idempotency(
      "/transactions",
      "core-key",
      json!({
        "source_account_id": cash_a.id,
        "destination_account_id": eq_a.id,
        "amount": "50.00",
        "description": "User A transfer"
      }),
    )
    .await;
  assert_eq!(status_a, StatusCode::CREATED);
  let txn_a_id = body_a["transaction_id"].as_i64().unwrap();

  // Create User B
  let (user_b, user_b_token) = harness.create_user("user_b_idemp", "pass_user_b").await;

  let cash_b = harness
    .create_account_for_user(user_b.id, "Cash B", AccountType::Asset)
    .await;
  let eq_b = harness
    .create_account_for_user(user_b.id, "Equity B", AccountType::Equity)
    .await;

  // Seed User B cash directly
  let seed_key = txn::core::types::IdempotencyKey::try_from("seed-key-b".to_string()).unwrap();
  let seed_amt = txn::core::types::Amount::try_from(dec!(300.00)).unwrap();
  let seed_desc = txn::core::types::Note::try_from("Seed B".to_string()).unwrap();
  let cmd = txn::core::types::TransferCmd {
    source_account_id: eq_b.id,
    destination_account_id: cash_b.id,
    amount: seed_amt,
    description: seed_desc,
  };
  txn::core::ledger::transfer(&harness.pool, user_b.id, &seed_key, &cmd)
    .await
    .unwrap();

  // User B submits WITH THE EXACT SAME KEY "core-key"
  let (status_b, body_b) = harness
    .request(
      axum::http::Method::POST,
      "/transactions",
      Some(&user_b_token),
      Some("core-key"),
      Some(json!({
        "source_account_id": cash_b.id,
        "destination_account_id": eq_b.id,
        "amount": "50.00",
        "description": "User B transfer"
      })),
    )
    .await;

  assert_eq!(status_b, StatusCode::CREATED);
  let txn_b_id = body_b["transaction_id"].as_i64().unwrap();

  // They must be independent transactions
  assert_ne!(txn_a_id, txn_b_id);
}
