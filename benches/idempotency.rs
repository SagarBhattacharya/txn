mod common;

use axum::http::StatusCode;
use common::{
  BenchClient, BenchResult, WorkerStats, benchmark_db_pool, print_results, verify_user_ledger,
};
use rust_decimal::dec;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Barrier;
use tokio::task::JoinSet;

const REPLAY_COUNT: usize = 10_000;
const RACE_WORKERS: usize = 50;

const INITIAL_BALANCE: &str = "1000000.00";
const TRANSFER_AMOUNT: &str = "1.00";

#[tokio::main]
async fn main() -> BenchResult<()> {
  println!("--- TXN Idempotency Benchmark ---");

  let client = BenchClient::new().await?;

  /*
   * ============================================================
   * 1. FRESH REQUEST
   * ============================================================
   *
   * Measure one normal transaction using a previously unused
   * idempotency key.
   */
  println!();
  println!("--- 1. Fresh Request ---");

  let equity = client
    .create_account("idempotency-fresh-equity", "Equity")
    .await?;

  let asset = client
    .create_account("idempotency-fresh-asset", "Asset")
    .await?;

  let funding = client
    .transfer(
      equity,
      asset,
      INITIAL_BALANCE,
      "setup-fresh-funding",
      "Idempotency benchmark setup",
    )
    .await?;

  if funding.status != StatusCode::CREATED {
    return Err(format!("fresh benchmark funding failed: {}", funding.status).into());
  }

  let started = Instant::now();

  let fresh = client
    .transfer(
      asset,
      equity,
      TRANSFER_AMOUNT,
      "idempotency-fresh-key",
      "Fresh idempotency request",
    )
    .await?;

  let fresh_elapsed = started.elapsed();

  if fresh.status != StatusCode::CREATED {
    return Err(format!("fresh request returned {}", fresh.status).into());
  }

  println!(
    "Fresh request latency: {:.2} ms",
    fresh_elapsed.as_secs_f64() * 1000.0
  );

  /*
   * ============================================================
   * 2. SEQUENTIAL REPLAY
   * ============================================================
   *
   * First create one transaction. Then repeatedly send the
   * exact same request and idempotency key.
   *
   * Every request should return 201, but no additional
   * transaction should be inserted.
   */
  println!();
  println!("--- 2. Sequential Replay ---");

  let equity = client
    .create_account("idempotency-replay-equity", "Equity")
    .await?;

  let asset = client
    .create_account("idempotency-replay-asset", "Asset")
    .await?;

  let funding = client
    .transfer(
      equity,
      asset,
      INITIAL_BALANCE,
      "setup-replay-funding",
      "Idempotency replay setup",
    )
    .await?;

  if funding.status != StatusCode::CREATED {
    return Err(format!("replay benchmark funding failed: {}", funding.status).into());
  }

  let replay_key = "idempotency-sequential-key";

  let first = client
    .transfer(
      asset,
      equity,
      TRANSFER_AMOUNT,
      replay_key,
      "Sequential replay target",
    )
    .await?;

  if first.status != StatusCode::CREATED {
    return Err(format!("initial replay request returned {}", first.status).into());
  }

  let started = Instant::now();
  let mut stats = WorkerStats::new();

  for _ in 0..REPLAY_COUNT {
    let response = client
      .transfer(
        asset,
        equity,
        TRANSFER_AMOUNT,
        replay_key,
        "Sequential replay target",
      )
      .await;

    stats.record(response);
  }

  let elapsed = started.elapsed();

  print_results("TXN SEQUENTIAL IDEMPOTENCY REPLAY", &stats, elapsed);

  if stats.successful() != REPLAY_COUNT as u64
    || stats.client_errors() != 0
    || stats.server_errors() != 0
    || stats.transport_errors != 0
  {
    return Err("sequential replay produced unexpected responses; see status distribution".into());
  }

  /*
   * The source balance should have decreased exactly once.
   */
  let asset_balance = client.get_balance(asset).await?;

  if asset_balance != dec!(999999.00) {
    return Err(format!("sequential replay changed balance incorrectly: {asset_balance}").into());
  }

  println!("Asset balance after {REPLAY_COUNT} replays: {asset_balance}");

  /*
   * ============================================================
   * 3. CONCURRENT SAME-KEY RACE
   * ============================================================
   *
   * No request is executed beforehand.
   *
   * All workers simultaneously attempt the exact same
   * transaction with the exact same idempotency key.
   *
   * Expected:
   *
   *     1 request -> fresh transaction
   *    49 requests -> replay
   *
   * But externally every request returns 201 with the same
   * transaction id.
   */
  println!();
  println!("--- 3. Concurrent Same-Key Race ---");

  let equity = client
    .create_account("idempotency-race-equity", "Equity")
    .await?;

  let asset = client
    .create_account("idempotency-race-asset", "Asset")
    .await?;

  let funding = client
    .transfer(
      equity,
      asset,
      INITIAL_BALANCE,
      "setup-race-funding",
      "Idempotency race setup",
    )
    .await?;

  if funding.status != StatusCode::CREATED {
    return Err(format!("race benchmark funding failed: {}", funding.status).into());
  }

  let bench = Arc::new(client);
  let barrier = Arc::new(Barrier::new(RACE_WORKERS + 1));
  let mut join_set = JoinSet::new();

  let race_key = "idempotency-concurrent-race";

  for _worker_id in 0..RACE_WORKERS {
    let bench = Arc::clone(&bench);
    let barrier = Arc::clone(&barrier);

    join_set.spawn(async move {
      let mut stats = WorkerStats::new();

      /*
       * Everybody reaches this point before any request
       * is allowed to execute.
       */
      barrier.wait().await;

      let response = bench
        .transfer(
          asset,
          equity,
          TRANSFER_AMOUNT,
          race_key,
          "Concurrent same-key race",
        )
        .await;

      stats.record(response);

      stats
    });
  }

  let started = Instant::now();

  // Release all workers simultaneously.
  barrier.wait().await;

  let mut combined = WorkerStats::new();

  while let Some(result) = join_set.join_next().await {
    combined.merge(result?);
  }

  let elapsed = started.elapsed();

  print_results("TXN CONCURRENT IDEMPOTENCY RACE", &combined, elapsed);

  let expected = RACE_WORKERS as u64;

  if combined.successful() != expected
    || combined.client_errors() != 0
    || combined.server_errors() != 0
    || combined.transport_errors != 0
  {
    return Err("concurrent idempotency race produced unexpected responses".into());
  }

  /*
   * ============================================================
   * DATABASE VERIFICATION
   * ============================================================
   *
   * Exactly one transaction should exist for the race key.
   */
  let db = benchmark_db_pool().await?;

  let transaction_id: i32 = sqlx::query_scalar(
    r#"
		SELECT id
		FROM transactions
		WHERE user_id = $1
		  AND idempotency_key = $2
		"#,
  )
  .bind(bench.user_id)
  .bind(race_key)
  .fetch_one(&db)
  .await?;

  let transaction_count: i64 = sqlx::query_scalar(
    r#"
		SELECT COUNT(*)
		FROM transactions
		WHERE user_id = $1
		  AND idempotency_key = $2
		"#,
  )
  .bind(bench.user_id)
  .bind(race_key)
  .fetch_one(&db)
  .await?;

  if transaction_count != 1 {
    return Err(
      format!("expected exactly 1 transaction for race key, found {transaction_count}").into(),
    );
  }

  let entry_count: i64 = sqlx::query_scalar(
    r#"
		SELECT COUNT(*)
		FROM entries
		WHERE transaction_id = $1
		"#,
  )
  .bind(transaction_id)
  .fetch_one(&db)
  .await?;

  if entry_count != 2 {
    return Err(
      format!("expected exactly 2 entries in idempotent transaction, found {entry_count}").into(),
    );
  }

  /*
   * Exactly one $1.00 transfer should have happened.
   */
  let final_balance = bench.get_balance(asset).await?;

  if final_balance != dec!(999999.00) {
    return Err(format!("concurrent replay changed balance incorrectly: {final_balance}").into());
  }

  let user_sum = verify_user_ledger(&db, bench.user_id).await?;

  if user_sum != dec!(0.00) {
    return Err(format!("ledger invariant failed: benchmark user sum = {user_sum}").into());
  }

  println!();
  println!("--- Concurrent Replay Correctness ---");
  println!("Requests:              {RACE_WORKERS}");
  println!("Successful:            {}", combined.successful());
  println!("Transactions created:  {transaction_count}");
  println!("Entries created:       {entry_count}");
  println!("Asset balance:         {final_balance}");
  println!("Ledger sum:            {user_sum}");
  println!("Idempotency invariant: PASS");
  println!();

  /*
   * Keep the first benchmark isolated too.
   */
  let fresh_db_sum = verify_user_ledger(&db, bench.user_id).await?;

  println!("Fresh/race benchmark user ledger sum: {fresh_db_sum}");

  Ok(())
}
