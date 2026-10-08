mod common;

use axum::http::StatusCode;
use common::{
  BenchClient, BenchResponse, BenchResult, WorkerStats, benchmark_db_pool, verify_user_ledger,
};
use rust_decimal::Decimal;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Barrier;
use tokio::task::JoinSet;

const WORKERS: usize = 16;
const READS_PER_WORKER: usize = 625;
const TOTAL_READS: usize = WORKERS * READS_PER_WORKER;

/*
 * Number of ledger entries belonging to the account being read.
 *
 * Each seeded transaction creates:
 *
 *     asset   +1.00
 *     equity  -1.00
 *
 * So N transactions means N entries for the Asset account.
 */
const DATASET_SIZES: &[i32] = &[100, 1_000, 10_000, 100_000];

#[tokio::main]
async fn main() -> BenchResult<()> {
  println!("--- TXN Balance Read Benchmark ---");
  println!("Workers:              {WORKERS}");
  println!("Reads / worker:       {READS_PER_WORKER}");
  println!("Reads / dataset:      {TOTAL_READS}");

  for &entry_count in DATASET_SIZES {
    run_dataset(entry_count).await?;
  }

  Ok(())
}

async fn run_dataset(entry_count: i32) -> BenchResult<()> {
  println!();
  println!("============================================================");
  println!("BALANCE READ BENCHMARK");
  println!("============================================================");
  println!("Entries on target:    {entry_count}");
  println!("Concurrent workers:   {WORKERS}");
  println!("Total reads:          {TOTAL_READS}");

  /*
   * ------------------------------------------------------------------------
   * Setup
   * ------------------------------------------------------------------------
   *
   * Use a fresh benchmark user for every dataset size so that the datasets
   * are isolated from one another.
   */
  let bench = BenchClient::new().await?;

  let equity = bench
    .create_account(&format!("balance-{entry_count}-equity"), "Equity")
    .await?;

  let asset = bench
    .create_account(&format!("balance-{entry_count}-asset"), "Asset")
    .await?;

  println!("Asset account:        {asset}");
  println!("Equity account:       {equity}");

  let pool = benchmark_db_pool().await?;

  /*
   * We seed directly in PostgreSQL.
   *
   * This is intentionally outside the measured region. The benchmark is
   * measuring GET /accounts/:id/balance, not the cost of constructing
   * 100,000 HTTP transactions.
   */
  println!("--- Seeding {entry_count} transactions ---");

  seed_ledger(&pool, bench.user_id, asset, equity, entry_count).await?;

  /*
   * Verify the fixture before measuring anything.
   */
  let seeded_balance = db_balance(&pool, asset).await?;

  let expected_balance = Decimal::from(entry_count);

  if seeded_balance != expected_balance {
    return Err(
      format!(
        "fixture balance mismatch: expected {expected_balance}, \
			 got {seeded_balance}"
      )
      .into(),
    );
  }

  let ledger_sum = verify_user_ledger(&pool, bench.user_id).await?;

  if ledger_sum != Decimal::ZERO {
    return Err(format!("fixture violates zero-sum invariant: {ledger_sum}").into());
  }

  println!("Seeded balance:       {seeded_balance}");
  println!("Ledger sum:           {ledger_sum}");

  /*
   * ------------------------------------------------------------------------
   * Warmup
   * ------------------------------------------------------------------------
   *
   * We deliberately do NOT alter the financial state during warmup.
   * These are reads only.
   *
   * This establishes/reuses HTTP connections and gets the server out of
   * its completely cold state before timing starts.
   */
  println!("--- Warmup ---");

  for _ in 0..32 {
    let response = get_balance_raw(&bench, asset).await?;

    if response.status != StatusCode::OK {
      return Err(format!("warmup returned {}", response.status).into());
    }
  }

  /*
   * ------------------------------------------------------------------------
   * Measured workload
   * ------------------------------------------------------------------------
   */

  println!("--- Starting measured workload ---");

  let bench = Arc::new(bench);
  let barrier = Arc::new(Barrier::new(WORKERS + 1));

  let mut tasks = JoinSet::new();

  for _worker_id in 0..WORKERS {
    let bench = Arc::clone(&bench);

    let barrier = Arc::clone(&barrier);

    tasks.spawn(async move {
      let mut stats = WorkerStats::new();

      /*
       * Ensure all workers have been spawned before any of them
       * starts issuing measured requests.
       */
      barrier.wait().await;

      for _ in 0..READS_PER_WORKER {
        let result = get_balance_raw(&bench, asset).await;

        stats.record(result);
      }

      stats
    });
  }

  /*
   * Start the timer immediately before releasing the workers.
   */
  let started = Instant::now();

  barrier.wait().await;

  let mut combined = WorkerStats::new();

  while let Some(result) = tasks.join_next().await {
    let stats = result?;
    combined.merge(stats);
  }

  let elapsed = started.elapsed();

  print_results(entry_count, &combined, elapsed);

  /*
   * ------------------------------------------------------------------------
   * Correctness
   * ------------------------------------------------------------------------
   *
   * Every request must have returned HTTP 200.
   */
  let successful = combined.successful_status(StatusCode::OK);

  let client_errors = combined.client_errors();

  let server_errors = combined.server_errors();

  let transport_errors = combined.transport_errors;

  if successful != TOTAL_READS as u64 {
    return Err(format!("expected {TOTAL_READS} successful reads, got {successful}").into());
  }

  if client_errors != 0 {
    return Err(format!("balance benchmark produced {client_errors} 4xx responses").into());
  }

  if server_errors != 0 {
    return Err(format!("balance benchmark produced {server_errors} 5xx responses").into());
  }

  if transport_errors != 0 {
    return Err(format!("balance benchmark produced {transport_errors} transport errors").into());
  }

  /*
   * Verify the public API after the benchmark.
   *
   * This is outside the timed region.
   */
  let final_balance = bench.get_balance(asset).await?;

  if final_balance != expected_balance {
    return Err(
      format!(
        "final balance mismatch: expected {expected_balance}, \
			 got {final_balance}"
      )
      .into(),
    );
  }

  let final_sum = verify_user_ledger(&pool, bench.user_id).await?;

  if final_sum != Decimal::ZERO {
    return Err(format!("final zero-sum invariant failed: {final_sum}").into());
  }

  println!();
  println!("Correctness");
  println!("  Expected balance:   {expected_balance}");
  println!("  Final balance:      {final_balance}");
  println!("  Ledger sum:         {final_sum}");
  println!("  Balance reads:      {successful}/{TOTAL_READS}");
  println!("  Result:              PASS");

  Ok(())
}

/*
 * ----------------------------------------------------------------------------
 * HTTP balance request
 * ----------------------------------------------------------------------------
 *
 * We use the same Reqwest Client that BenchClient uses for all benchmarks.
 *
 * This returns a BenchResponse so WorkerStats can record:
 *
 *     - HTTP status
 *     - request latency
 */
async fn get_balance_raw(
  bench: &BenchClient,
  account_id: i32,
) -> Result<BenchResponse, reqwest::Error> {
  let started = Instant::now();

  let response = bench
    .client
    .get(format!("{}/accounts/{account_id}/balance", bench.base_url))
    .bearer_auth(&bench.token)
    .send()
    .await;

  match response {
    Ok(response) => {
      let status = response.status();

      /*
       * Consume the response body so Reqwest can reuse the
       * underlying connection.
       */
      let _body = response.bytes().await?;

      Ok(BenchResponse {
        status,
        latency: started.elapsed(),
        transaction_id: None,
      })
    }

    Err(error) => Err(error),
  }
}

/*
 * ----------------------------------------------------------------------------
 * Database fixture
 * ----------------------------------------------------------------------------
 *
 * Create `count` balanced transactions directly in PostgreSQL.
 *
 * Each transaction:
 *
 *     Asset   +1.00
 *     Equity  -1.00
 *
 * Therefore:
 *
 *     Asset balance  = count
 *     Equity balance = -count
 *     Global sum      = 0
 *
 * This is setup only and is not part of the benchmark timing.
 */
async fn seed_ledger(
  pool: &sqlx::PgPool,
  user_id: i32,
  asset_id: i32,
  equity_id: i32,
  count: i32,
) -> BenchResult<()> {
  let mut tx = pool.begin().await?;

  sqlx::query(
    r#"
		WITH seeded_transactions AS (
			INSERT INTO transactions (
				user_id,
				description,
				idempotency_key,
				request_hash
			)
			SELECT
				$1,
				'Balance benchmark seed',
				'read-bench-' || g,
				decode(
					repeat('00', 32),
					'hex'
				)
			FROM generate_series(1, $2) AS g
			RETURNING id
		)
		INSERT INTO entries (
			transaction_id,
			account_id,
			amount
		)
		SELECT
			id,
			$3,
			1.00
		FROM seeded_transactions

		UNION ALL

		SELECT
			id,
			$4,
			-1.00
		FROM seeded_transactions
		"#,
  )
  .bind(user_id)
  .bind(count)
  .bind(asset_id)
  .bind(equity_id)
  .execute(&mut *tx)
  .await?;

  tx.commit().await?;

  Ok(())
}

/*
 * ----------------------------------------------------------------------------
 * Direct DB balance check
 * ----------------------------------------------------------------------------
 *
 * This is only used to verify the fixture. The measured benchmark uses the
 * HTTP endpoint.
 */
async fn db_balance(pool: &sqlx::PgPool, account_id: i32) -> BenchResult<Decimal> {
  let balance = sqlx::query_scalar::<_, Decimal>(
    r#"
			SELECT COALESCE(
				SUM(amount),
				0.0
			)
			FROM entries
			WHERE account_id = $1
			"#,
  )
  .bind(account_id)
  .fetch_one(pool)
  .await?;

  Ok(balance)
}

/*
 * ----------------------------------------------------------------------------
 * Result formatting
 * ----------------------------------------------------------------------------
 *
 * We don't use the existing common::print_results() here because that helper
 * assumes HTTP 201 means success, which is correct for transfers/reversals
 * but incorrect for this GET endpoint.
 */
fn print_results(entry_count: i32, stats: &WorkerStats, elapsed: std::time::Duration) {
  let total = stats.total_requests();

  let throughput = total as f64 / elapsed.as_secs_f64();

  println!();
  println!("------------------- Results -------------------");

  println!("History entries:     {entry_count}");

  println!("Requests:            {total}");

  println!(
    "Successful (200):    {}",
    stats.successful_status(StatusCode::OK)
  );

  println!("4xx responses:       {}", stats.client_errors());

  println!("5xx responses:       {}", stats.server_errors());

  println!("Transport errors:    {}", stats.transport_errors);

  println!("Total time:          {:.2}s", elapsed.as_secs_f64());

  println!("Throughput:          {:.2} req/sec", throughput);

  if !stats.histogram.is_empty() {
    println!();
    println!("Latency");

    println!(
      "  p50:                {:.2} ms",
      stats.histogram.value_at_quantile(0.50) as f64 / 1000.0
    );

    println!(
      "  p90:                {:.2} ms",
      stats.histogram.value_at_quantile(0.90) as f64 / 1000.0
    );

    println!(
      "  p95:                {:.2} ms",
      stats.histogram.value_at_quantile(0.95) as f64 / 1000.0
    );

    println!(
      "  p99:                {:.2} ms",
      stats.histogram.value_at_quantile(0.99) as f64 / 1000.0
    );

    println!(
      "  max:                {:.2} ms",
      stats.histogram.max() as f64 / 1000.0
    );
  }

  println!("------------------------------------------------");
}
