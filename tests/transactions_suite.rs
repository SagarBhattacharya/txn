mod common;

use axum::http::StatusCode;
use common::TestHarness;
use rust_decimal::dec;
use serde_json::json;
use sqlx::PgPool;
use txn::db::queries::Query;
use txn::db::rows::AccountType;

#[sqlx::test]
async fn test_successful_transfer(pool: PgPool) {
  let harness = TestHarness::new(pool).await;
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
  let harness = TestHarness::new(pool).await;
  let (cash, equity) = harness
    .create_funded_pair("Checking", "Equity", dec!(100.00))
    .await;

  let payload = json!({
    "source_account_id": cash.id,
    "destination_account_id": equity.id,
    "amount": "150.00", // overdraft attempt
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
  let harness = TestHarness::new(pool).await;
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
  let harness = TestHarness::new(pool).await;
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
  let harness = TestHarness::new(pool).await;
  let (cash, equity) = harness
    .create_funded_pair("Cash", "Equity", dec!(500.00))
    .await;

  let payload = json!({
    "source_account_id": cash.id,
    "destination_account_id": equity.id,
    "amount": "50.00",
    "description": "Missing header transfer"
  });

  let (status, body) = harness
    .post_without_idempotency_key("/transactions", payload)
    .await;

  assert_eq!(status, StatusCode::BAD_REQUEST);
  assert_eq!(body["error"], "idempotency-key not found in headers");
}

#[sqlx::test]
async fn test_reversal_missing_idempotency_key_fails(pool: PgPool) {
  let harness = TestHarness::new(pool).await;
  let (cash, equity) = harness
    .create_funded_pair("Cash", "Equity", dec!(500.00))
    .await;

  let payload = json!({
    "source_account_id": cash.id,
    "destination_account_id": equity.id,
    "amount": "100.00",
    "description": "Pre-reversal transfer"
  });

  let (_, body) = harness.post_json("/transactions", payload).await;
  let txn_id = body["transaction_id"].as_i64().unwrap();

  let (status, err_body) = harness
    .post_without_idempotency_key(
      &format!("/transactions/{txn_id}/reversal"),
      json!({ "reason": "Accidental charge" }),
    )
    .await;

  assert_eq!(status, StatusCode::BAD_REQUEST);
  assert_eq!(err_body["error"], "idempotency-key not found in headers");
}

#[sqlx::test]
async fn test_unauthenticated_transaction_rejected(pool: PgPool) {
  let harness = TestHarness::new(pool).await;
  let (cash, equity) = harness
    .create_funded_pair("Cash", "Equity", dec!(100.00))
    .await;

  let payload = json!({
    "source_account_id": cash.id,
    "destination_account_id": equity.id,
    "amount": "10.00",
    "description": "Unauthenticated transfer"
  });

  let (status, _) = harness.post_unauthenticated("/transactions", payload).await;
  assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn test_cannot_transfer_from_other_users_account(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  // 1. Create User B with an account
  let (user_b, _) = harness.create_user("user_b_victim", "pass_b_9999").await;

  let victim_account = harness
    .create_account_for_user(user_b.id, "Victim Vault", AccountType::Asset)
    .await;

  let (attacker_dest, _) = harness
    .create_funded_pair("Attacker Cash", "Attacker Equity", dec!(0.00))
    .await;

  // 2. Default user (attacker) attempts to transfer money FROM victim_account
  let payload = json!({
    "source_account_id": victim_account.id,
    "destination_account_id": attacker_dest.id,
    "amount": "50.00",
    "description": "Unauthorized drain attempt"
  });

  let (status, body) = harness.post_json("/transactions", payload).await;
  assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test]
async fn test_cannot_reverse_foreign_transaction(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  // 1. Default user executes a valid transfer
  let (cash, equity) = harness
    .create_funded_pair("Cash", "Equity", dec!(200.00))
    .await;

  let payload = json!({
    "source_account_id": cash.id,
    "destination_account_id": equity.id,
    "amount": "50.00",
    "description": "Legit payment"
  });

  let (status, body) = harness.post_json("/transactions", payload).await;
  assert_eq!(status, StatusCode::CREATED);
  let txn_id = body["transaction_id"].as_i64().unwrap();

  // 2. Create User B (outsider)
  let (_, user_b_token) = harness.create_user("outsider_user", "pass_b_9999").await;

  // 3. User B attempts to reverse Default user's transaction
  let (rev_status, _) = harness
    .request(
      axum::http::Method::POST,
      &format!("/transactions/{txn_id}/reversal"),
      Some(&user_b_token),
      Some("rev-key-outsider"),
      Some(json!({ "reason": "Illegitimate reversal request" })),
    )
    .await;

  assert_eq!(rev_status, StatusCode::FORBIDDEN);
}

#[sqlx::test]
async fn test_transfer_to_non_existent_destination_fails(pool: PgPool) {
  let harness = TestHarness::new(pool).await;
  let (cash, _) = harness
    .create_funded_pair("Cash", "Equity", dec!(100.00))
    .await;

  let payload = json!({
    "source_account_id": cash.id,
    "destination_account_id": 999_999, // non-existent
    "amount": "25.00",
    "description": "Transfer to nowhere"
  });

  let (status, body) = harness.post_json("/transactions", payload).await;
  assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn test_transfer_to_another_users_account_succeeds(pool: PgPool) {
  let harness = TestHarness::new(pool).await;

  // 1. Default user owns funded source account
  let (cash, _) = harness
    .create_funded_pair("Payer Checking", "Payer Equity", dec!(300.00))
    .await;

  // 2. User B owns destination account

  let (user_b, user_b_token) = harness.create_user("recipient_user", "pass_b_9999").await;

  let recipient_account = harness
    .create_account_for_user(user_b.id, "Recipient Wallet", AccountType::Asset)
    .await;

  // 3. Payer sends funds to Recipient Wallet (P2P payment)
  let payload = json!({
    "source_account_id": cash.id,
    "destination_account_id": recipient_account.id,
    "amount": "120.00",
    "description": "Split dinner bill"
  });

  let (status, body) = harness.post_json("/transactions", payload).await;
  assert_eq!(status, StatusCode::CREATED);
  assert!(body["transaction_id"].is_i64());

  // 4. Verify Payer was debited
  let (_, payer_bal) = harness.get(&format!("/accounts/{}/balance", cash.id)).await;
  assert_eq!(payer_bal["balance"], "180.00");

  // 5. Verify Recipient was credited directly in DB
  let recipient_bal = Query::get_balance(&harness.pool, recipient_account.id)
    .await
    .unwrap();
  assert_eq!(recipient_bal, dec!(120.00));
}

#[sqlx::test]
async fn test_reject_sub_cent_precision_amount(pool: PgPool) {
  let harness = TestHarness::new(pool).await;
  let (cash, equity) = harness
    .create_funded_pair("CashSubCent", "EquitySubCent", dec!(1000.00))
    .await;

  // Attempt transfer with 3 decimal places (0.004)
  let payload = json!({
    "source_account_id": cash.id,
    "destination_account_id": equity.id,
    "amount": "0.004",
    "description": "Sub-cent transfer"
  });

  let (status, body) = harness.post_json("/transactions", payload).await;
  assert_eq!(status, StatusCode::BAD_REQUEST);
  assert!(
    body["error"]
      .as_str()
      .unwrap()
      .contains("cannot exceed 2 decimal places")
  );
}

#[sqlx::test]
async fn test_reject_amount_exceeding_numeric_12_2_limit(pool: PgPool) {
  let harness = TestHarness::new(pool).await;
  let (cash, equity) = harness
    .create_funded_pair("CashOverflow", "EquityOverflow", dec!(1000.00))
    .await;

  // Value has 11 integer digits, which exceeds numeric(12,2)
  let payload = json!({
    "source_account_id": cash.id,
    "destination_account_id": equity.id,
    "amount": "99999999999.00",
    "description": "Overflow transfer"
  });

  let (status, body) = harness.post_json("/transactions", payload).await;
  assert_eq!(status, StatusCode::BAD_REQUEST);
  assert!(
    body["error"]
      .as_str()
      .unwrap()
      .contains("exceeds maximum allowed limit")
  );
}

#[sqlx::test]
async fn test_database_enforces_append_only_immutability(pool: PgPool) {
  let harness = TestHarness::new(pool.clone()).await;
  let (cash, equity) = harness
    .create_funded_pair("CashImmut", "EquityImmut", dec!(500.00))
    .await;

  let (_, body) = harness
    .post_json(
      "/transactions",
      json!({
        "source_account_id": cash.id,
        "destination_account_id": equity.id,
        "amount": "100.00",
        "description": "Immutable transfer"
      }),
    )
    .await;

  let txn_id = body["transaction_id"].as_i64().unwrap() as i32;

  // 1. Attempt raw SQL UPDATE on transactions table
  let update_res = sqlx::query!(
    "UPDATE transactions SET description = 'Tampered' WHERE id = $1",
    txn_id
  )
  .execute(&pool)
  .await;

  assert!(update_res.is_err(), "UPDATE on transactions must fail");
  let err_str = update_res.unwrap_err().to_string();
  println!("ACTUAL DB ERROR: {err_str}");
  assert!(err_str.contains("db records are immutable"));

  // 2. Attempt raw SQL DELETE on entries table
  let delete_res = sqlx::query!("DELETE FROM entries WHERE transaction_id = $1", txn_id)
    .execute(&pool)
    .await;

  assert!(delete_res.is_err(), "DELETE on entries must fail");
  let err_str = delete_res.unwrap_err().to_string();
  assert!(err_str.contains("db records are immutable"));
}
