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

const WORKERS: usize = 16;
const OPERATIONS_PER_WORKER: usize = 250;
const TOTAL_OPERATIONS: usize = WORKERS * OPERATIONS_PER_WORKER;

const INITIAL_BALANCE: &str = "10000.00";
const TRANSFER_AMOUNT: &str = "10.00";

const RACE_WORKERS: usize = 50;

#[tokio::main]
async fn main() -> BenchResult<()> {
  println!("--- TXN Reversal Benchmark ---");

  independent_reversals().await?;
  same_target_race().await?;

  Ok(())
}

async fn independent_reversals() -> BenchResult<()> {
  println!();
  println!("============================================================");
  println!("1. INDEPENDENT REVERSAL THROUGHPUT");
  println!("============================================================");

  println!("Workers:              {WORKERS}");
  println!("Operations / worker:  {OPERATIONS_PER_WORKER}");
  println!("Total operations:     {TOTAL_OPERATIONS}");

  let bench = BenchClient::new().await?;

  println!("--- Setting up independent accounts ---");

  let funding_equity = bench.create_account("reversal-equity", "Equity").await?;

  let mut targets = Vec::with_capacity(TOTAL_OPERATIONS);

  /*
   * Each worker receives its own source/destination pair.
   *
   * Therefore the measured reversals don't intentionally
   * contend on the same account rows.
   */
  let mut pairs = Vec::with_capacity(WORKERS);

  for worker_id in 0..WORKERS {
    let source = bench
      .create_account(&format!("reversal-source-{worker_id}"), "Asset")
      .await?;

    let destination = bench
      .create_account(&format!("reversal-destination-{worker_id}"), "Asset")
      .await?;

    let funding = bench
      .transfer(
        funding_equity,
        source,
        INITIAL_BALANCE,
        &format!("setup-reversal-funding-{worker_id}"),
        "Reversal benchmark funding",
      )
      .await?;

    if funding.status != StatusCode::CREATED {
      return Err(format!("funding failed for worker {worker_id}: {}", funding.status).into());
    }

    pairs.push((source, destination));
  }

  println!("--- Creating transactions to reverse ---");

  /*
   * Create all transactions before the measured region.
   *
   * Every transaction is:
   *
   *     source      -10.00
   *     destination +10.00
   *
   * Each transaction is an independent reversal target.
   */
  for operation in 0..TOTAL_OPERATIONS {
    let worker_id = operation % WORKERS;

    let (source, destination) = pairs[worker_id];

    let transfer = bench
      .transfer(
        source,
        destination,
        TRANSFER_AMOUNT,
        &format!("setup-reversal-target-{operation}"),
        "Reversal benchmark target",
      )
      .await?;

    if transfer.status != StatusCode::CREATED {
      return Err(
        format!(
          "failed to create reversal target {operation}: {}",
          transfer.status
        )
        .into(),
      );
    }

    let transaction_id = transfer
      .transaction_id
      .ok_or("successful transfer did not return transaction_id")?;

    targets.push(transaction_id);
  }

  println!("Created {TOTAL_OPERATIONS} reversal targets.");

  /*
   * Warmup an HTTP connection for each worker.
   *
   * We create a real reversal here, but it targets one of the
   * already-created benchmark transactions, so it must not be
   * counted twice.
   *
   * To avoid changing the measured dataset, use a completely
   * separate setup transaction for warmup.
   */
  println!("--- Warmup ---");

  for (worker_id, value) in pairs.iter().enumerate().take(WORKERS) {
    let (source, destination) = *value;

    let warmup_transfer = bench
      .transfer(
        source,
        destination,
        "1.00",
        &format!("reversal-warmup-target-{worker_id}"),
        "Reversal benchmark warmup",
      )
      .await?;

    if warmup_transfer.status != StatusCode::CREATED {
      return Err(
        format!(
          "warmup transfer failed for worker {worker_id}: {}",
          warmup_transfer.status
        )
        .into(),
      );
    }

    let warmup_id = warmup_transfer
      .transaction_id
      .ok_or("warmup transfer missing transaction_id")?;

    let warmup_reversal = bench
      .reverse(
        warmup_id,
        "Warmup reversal",
        &format!("reversal-warmup-{worker_id}"),
      )
      .await?;

    if warmup_reversal.status != StatusCode::CREATED {
      return Err(
        format!(
          "warmup reversal failed for worker {worker_id}: {}",
          warmup_reversal.status
        )
        .into(),
      );
    }
  }

  println!("--- Starting measured workload ---");

  let bench = Arc::new(bench);
  let targets = Arc::new(targets);

  let barrier = Arc::new(Barrier::new(WORKERS + 1));
  let mut join_set = JoinSet::new();

  for worker_id in 0..WORKERS {
    let bench = Arc::clone(&bench);
    let targets = Arc::clone(&targets);
    let barrier = Arc::clone(&barrier);

    join_set.spawn(async move {
      let mut stats = WorkerStats::new();

      barrier.wait().await;

      for operation in 0..OPERATIONS_PER_WORKER {
        let index = worker_id * OPERATIONS_PER_WORKER + operation;

        let target = targets[index];

        let result = bench
          .reverse(
            target,
            "Independent benchmark reversal",
            &format!("reversal-{worker_id}-{operation}"),
          )
          .await;

        stats.record(result);
      }

      stats
    });
  }

  let started = Instant::now();

  barrier.wait().await;

  let mut combined = WorkerStats::new();

  while let Some(result) = join_set.join_next().await {
    let stats = result?;
    combined.merge(stats);
  }

  let elapsed = started.elapsed();

  print_results("TXN INDEPENDENT REVERSAL THROUGHPUT", &combined, elapsed);

  /*
   * Every target should have exactly one reversal.
   */
  if combined.successful() != TOTAL_OPERATIONS as u64
    || combined.client_errors() != 0
    || combined.server_errors() != 0
    || combined.transport_errors != 0
  {
    return Err("independent reversal benchmark produced unexpected responses".into());
  }

  /*
   * Verify a sample of balances.
   *
   * Because every transfer was immediately reversed, every
   * benchmark source should be back at 1000.00 and every
   * destination should be 10.00.
   */
  println!("--- Verifying balances ---");

  for &(source, destination) in &pairs {
    let source_balance = bench.get_balance(source).await?;
    let destination_balance = bench.get_balance(destination).await?;

    if source_balance != rust_decimal::dec!(10000.00) {
      return Err(format!("source balance incorrect: {source_balance}").into());
    }

    if destination_balance != rust_decimal::dec!(0.00) {
      return Err(format!("destination balance incorrect: {destination_balance}").into());
    }
  }

  let db = benchmark_db_pool().await?;
  let user_sum = verify_user_ledger(&db, bench.user_id).await?;

  println!("Benchmark user ledger sum: {user_sum}");

  if user_sum != dec!(0.00) {
    return Err(format!("ledger invariant failed: {user_sum}").into());
  }

  println!("Independent reversal correctness: PASS");

  Ok(())
}

async fn same_target_race() -> BenchResult<()> {
  println!();
  println!("============================================================");
  println!("2. CONCURRENT SAME-TARGET REVERSAL");
  println!("============================================================");

  println!("Concurrent reversal requests: {RACE_WORKERS}");

  let bench = BenchClient::new().await?;

  println!("--- Setting up target transaction ---");

  let equity = bench.create_account("race-equity", "Equity").await?;

  let source = bench.create_account("race-source", "Asset").await?;

  let destination = bench.create_account("race-destination", "Asset").await?;

  let funding = bench
    .transfer(
      equity,
      source,
      "1000.00",
      "setup-race-funding",
      "Reversal race setup",
    )
    .await?;

  if funding.status != StatusCode::CREATED {
    return Err(format!("race funding failed: {}", funding.status).into());
  }

  /*
   * The target transaction is:
   *
   *     source       -10
   *     destination  +10
   */
  let target = bench
    .transfer(
      source,
      destination,
      "10.00",
      "reversal-race-target",
      "Concurrent reversal target",
    )
    .await?;

  if target.status != StatusCode::CREATED {
    return Err(format!("target transfer failed: {}", target.status).into());
  }

  let target_id = target
    .transaction_id
    .ok_or("target transfer missing transaction_id")?;

  println!("Target transaction: {target_id}");

  /*
   * All workers attempt to reverse EXACTLY the same
   * transaction, but each uses a different idempotency key.
   *
   * That distinction matters.
   *
   * Same key would test idempotency.
   * Different keys test the reversal target lock.
   */
  println!("--- Starting concurrent reversal race ---");

  let bench = Arc::new(bench);
  let barrier = Arc::new(Barrier::new(RACE_WORKERS + 1));

  let mut join_set = JoinSet::new();

  for worker_id in 0..RACE_WORKERS {
    let bench = Arc::clone(&bench);
    let barrier = Arc::clone(&barrier);

    join_set.spawn(async move {
      let mut stats = WorkerStats::new();

      barrier.wait().await;

      let result = bench
        .reverse(
          target_id,
          &format!("Concurrent reversal {worker_id}"),
          &format!("reversal-race-key-{worker_id}"),
        )
        .await;

      stats.record(result);

      stats
    });
  }

  let started = Instant::now();

  barrier.wait().await;

  let mut combined = WorkerStats::new();

  while let Some(result) = join_set.join_next().await {
    let stats = result?;
    combined.merge(stats);
  }

  let elapsed = started.elapsed();

  print_results("TXN SAME-TARGET REVERSAL RACE", &combined, elapsed);

  /*
   * Exactly one request should succeed.
   *
   * The other 49 should receive 422 AlreadyReversed.
   */
  let successful = combined.successful();

  let already_reversed = combined
    .statuses
    .get(&StatusCode::UNPROCESSABLE_ENTITY.as_u16())
    .copied()
    .unwrap_or(0);

  println!();
  println!("Reversal race correctness");
  println!("  Created:             {successful}");
  println!("  422 responses:       {already_reversed}");
  println!("  Expected Created:    1");
  println!("  Expected 422:        {}", RACE_WORKERS - 1);

  if successful != 1 {
    return Err(format!("expected exactly one successful reversal, got {successful}").into());
  }

  if already_reversed != (RACE_WORKERS - 1) as u64 {
    return Err(
      format!(
        "expected {} AlreadyReversed responses, got {already_reversed}",
        RACE_WORKERS - 1
      )
      .into(),
    );
  }

  if combined.server_errors() != 0 || combined.transport_errors != 0 {
    return Err("reversal race produced server or transport failures".into());
  }

  /*
   * The source was debited 10.00 and then restored exactly once.
   */
  let source_balance = bench.get_balance(source).await?;
  let destination_balance = bench.get_balance(destination).await?;

  println!();
  println!("Final balances");
  println!("  Source:              {source_balance}");
  println!("  Destination:         {destination_balance}");

  if source_balance != dec!(1000.00) {
    return Err(format!("source balance incorrect: {source_balance}").into());
  }

  if destination_balance != dec!(0.00) {
    return Err(format!("destination balance incorrect: {destination_balance}").into());
  }

  /*
   * Confirm exactly one reversal points back to the target.
   */
  let db = benchmark_db_pool().await?;

  let reversal_count: i64 = sqlx::query_scalar(
    r#"
		SELECT COUNT(*)
		FROM transactions
		WHERE reversed_transaction_id = $1
		"#,
  )
  .bind(target_id)
  .fetch_one(&db)
  .await?;

  if reversal_count != 1 {
    return Err(
      format!("expected exactly one reversal transaction, found {reversal_count}").into(),
    );
  }

  let user_sum = verify_user_ledger(&db, bench.user_id).await?;

  println!("Benchmark user ledger sum: {user_sum}");

  if user_sum != dec!(0.00) {
    return Err(format!("ledger invariant failed: {user_sum}").into());
  }

  println!();
  println!("Same-target reversal invariant: PASS");

  Ok(())
}
