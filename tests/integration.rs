mod suite;

use axum::http::{Method, StatusCode};
use suite::TestHarness;
use rust_decimal::{dec, Decimal};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::task::JoinSet;
use txn::db::rows::AccountType;

#[sqlx::test]
async fn test_auth_and_account_uniqueness_invariants(pool: PgPool) {
	let harness = TestHarness::new(pool).await;

	// 1. User registration & duplicate username rejection
	let (status, body) = harness
		.request_raw(
			Method::POST,
			"/users/register",
			None,
			None,
			Some(json!({ "username": "alice", "password": "supersecurepassword123" })),
		)
		.await;
	assert_eq!(status, StatusCode::CREATED);
	let alice_token = body["token"].as_str().unwrap();

	let (dup_status, dup_body) = harness
		.request_raw(
			Method::POST,
			"/users/register",
			None,
			None,
			Some(json!({ "username": "alice", "password": "supersecurepassword123" })),
		)
		.await;
	assert_eq!(dup_status, StatusCode::CONFLICT);
	assert_eq!(dup_body["error"], "Username already taken");

	// 2. Account name unique per user, but allowed across different users
	let alice = harness.create_user("alice_acc", "password1234").await;
	let bob = harness.create_user("bob_acc", "password1234").await;

	let (s1, _) = alice.create_account("Operating Cash", AccountType::Asset).await;
	assert_eq!(s1, StatusCode::CREATED);

	let (s_conflict, _) = alice.create_account("Operating Cash", AccountType::Asset).await;
	assert_eq!(s_conflict, StatusCode::CONFLICT);

	let (s2, _) = bob.create_account("Operating Cash", AccountType::Asset).await;
	assert_eq!(s2, StatusCode::CREATED);
}

#[sqlx::test]
async fn test_multi_tenant_isolation_invariants(pool: PgPool) {
	let harness = TestHarness::new(pool).await;
	let alice = harness.create_user("alice_tenant", "password1234").await;
	let bob = harness.create_user("bob_tenant", "password1234").await;

	let (alice_cash, alice_eq) = alice.create_funded_pair("Cash", "Equity", "200.00").await;
	let (bob_cash, _) = bob.create_funded_pair("Cash", "Equity", "0.00").await;

	// Bob cannot read Alice's account or balance
	let (s_get, _) = bob.get_account(alice_cash).await;
	assert_eq!(s_get, StatusCode::FORBIDDEN);

	// Bob cannot debit Alice's account
	let (s_steal, _) = bob.transfer(alice_cash, bob_cash, "50.00", "Theft attempt", None).await;
	assert_eq!(s_steal, StatusCode::FORBIDDEN);

	// Alice transfers legitimately to Bob (P2P payment)
	let (s_p2p, p2p_body) = alice.transfer(alice_cash, bob_cash, "50.00", "Lunch", None).await;
	assert_eq!(s_p2p, StatusCode::CREATED);
	let txn_id = p2p_body["transaction_id"].as_i64().unwrap();

	// Outsider Charlie cannot reverse Alice's transaction
	let charlie = harness.create_user("charlie", "password1234").await;
	let (s_rev_foreign, _) = charlie.reverse(txn_id, "Unauthorized reverse", None).await;
	assert_eq!(s_rev_foreign, StatusCode::FORBIDDEN);
}

#[sqlx::test]
async fn test_transfer_and_overdraft_invariants(pool: PgPool) {
	let harness = TestHarness::new(pool).await;
	let user = harness.create_user("trader", "password1234").await;
	let (cash, equity) = user.create_funded_pair("Cash", "Equity", "100.00").await;

	// Overdraft rejection for Asset accounts
	let (s_overdraft, _) = user.transfer(cash, equity, "100.01", "Overdraft", None).await;
	assert_eq!(s_overdraft, StatusCode::UNPROCESSABLE_ENTITY);
	assert_eq!(user.get_balance(cash).await, dec!(100.00));

	// Successful debit
	let (s_ok, _) = user.transfer(cash, equity, "40.00", "Legit transfer", None).await;
	assert_eq!(s_ok, StatusCode::CREATED);
	assert_eq!(user.get_balance(cash).await, dec!(60.00));

	// History endpoint verification
	let (s_hist, hist_body) = user
		.harness
		.request_raw(Method::GET, &format!("/accounts/{cash}/transactions"), Some(&user.token), None, None)
		.await;
	assert_eq!(s_hist, StatusCode::OK);
	assert_eq!(hist_body.as_array().unwrap().len(), 2); // 1 seed + 1 transfer
}

#[sqlx::test]
async fn test_idempotency_lifecycle_and_concurrency(pool: PgPool) {
	let harness = Arc::new(TestHarness::new(pool).await);
	let alice = harness.create_user("alice_idemp", "password1234").await;
	let (cash, equity) = alice.create_funded_pair("Cash", "Equity", "1000.00").await;

	let key = "idemp-test-key-100";

	// 1. Initial execution
	let (s1, b1) = alice.transfer(cash, equity, "100.00", "Transfer 1", Some(key)).await;
	assert_eq!(s1, StatusCode::CREATED);
	let txn_id = b1["transaction_id"].as_i64().unwrap();

	// 2. Replay with identical payload returns cached txn_id without re-debiting
	let (s2, b2) = alice.transfer(cash, equity, "100.00", "Transfer 1", Some(key)).await;
	assert_eq!(s2, StatusCode::CREATED);
	assert_eq!(b2["transaction_id"].as_i64().unwrap(), txn_id);
	assert_eq!(alice.get_balance(cash).await, dec!(900.00));

	// 3. Replay with modified payload yields 422 Conflict Mismatch
	let (s3, b3) = alice.transfer(cash, equity, "200.00", "Transfer 1", Some(key)).await;
	assert_eq!(s3, StatusCode::UNPROCESSABLE_ENTITY);
	assert!(b3["error"].as_str().unwrap().contains("mismatched"));

	// 4. Concurrent race with same idempotency key executes exactly once
	let shared_key = "idemp-race-key";
	let mut handles = JoinSet::new();
	for _ in 0..10 {
		let a = alice.clone();
		handles.spawn(async move {
			a.transfer(cash, equity, "10.00", "Race", Some(shared_key)).await
		});
	}

	let mut ids = Vec::new();
	while let Some(res) = handles.join_next().await {
		let (status, body) = res.unwrap();
		assert!(status == StatusCode::CREATED || status == StatusCode::OK);
		ids.push(body["transaction_id"].as_i64().unwrap());
	}
	assert!(ids.iter().all(|&id| id == ids[0]));
	assert_eq!(alice.get_balance(cash).await, dec!(890.00)); // Exactly one 10.00 deduction
}

#[sqlx::test]
async fn test_reversal_lifecycle_and_concurrency(pool: PgPool) {
	let harness = Arc::new(TestHarness::new(pool).await);
	let user = harness.create_user("rev_user", "password1234").await;
	let (cash, equity) = user.create_funded_pair("Cash", "Equity", "500.00").await;

	let (_, b) = user.transfer(cash, equity, "150.00", "Spend", None).await;
	let txn_id = b["transaction_id"].as_i64().unwrap();

	// Concurrent double reversal race: exactly one must win (201), other gets 422
	let u1 = user.clone();
	let u2 = user.clone();
	let t1 = tokio::spawn(async move { u1.reverse(txn_id, "Race 1", Some("rev-key-1")).await });
	let t2 = tokio::spawn(async move { u2.reverse(txn_id, "Race 2", Some("rev-key-2")).await });

	let (r1, r2) = tokio::join!(t1, t2);
	let (s1, _) = r1.unwrap();
	let (s2, _) = r2.unwrap();

	let statuses = vec![s1, s2];
	println!("{:?}", statuses);
	assert!(statuses.contains(&StatusCode::CREATED));
	assert!(statuses.contains(&StatusCode::UNPROCESSABLE_ENTITY));

	// Funds restored back to 500.00 without double refund
	assert_eq!(user.get_balance(cash).await, dec!(500.00));
}

#[sqlx::test]
async fn test_database_immutability_and_money_conservation(pool: PgPool) {
	let harness = TestHarness::new(pool.clone()).await;
	let user = harness.create_user("audit_user", "password1234").await;
	let (cash, equity) = user.create_funded_pair("Cash", "Equity", "300.00").await;

	let (_, b) = user.transfer(cash, equity, "50.00", "Audit transfer", None).await;
	let txn_id = b["transaction_id"].as_i64().unwrap();

	// 1. Immutability trigger checks
	let update_res = sqlx::query("UPDATE transactions SET description = 'Tampered' WHERE id = $1")
		.bind(txn_id as i32)
		.execute(&pool)
		.await;
	assert!(update_res.unwrap_err().to_string().contains("db records are immutable"));

	let delete_res = sqlx::query("DELETE FROM entries WHERE transaction_id = $1")
		.bind(txn_id as i32)
		.execute(&pool)
		.await;
	assert!(delete_res.unwrap_err().to_string().contains("db records are immutable"));

	// 2. Global conservation of money: SUM(amount) in entries MUST be exactly 0
	let total_sum: Decimal = sqlx::query_scalar("SELECT COALESCE(SUM(amount), 0) FROM entries")
		.fetch_one(&pool)
		.await
		.unwrap();
	assert_eq!(total_sum, dec!(0.00));
}