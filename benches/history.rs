mod common;

use axum::http::StatusCode;
use common::{BenchClient, BenchResult, WorkerStats, benchmark_db_pool, verify_user_ledger};
use rust_decimal::Decimal;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Barrier;
use tokio::task::JoinSet;

/*
 * Keep this identical to reads.rs so the two benchmarks
 * can be compared directly.
 */
const WORKERS: usize = 16;
const READS_PER_WORKER: usize = 625;
const TOTAL_READS: usize = WORKERS * READS_PER_WORKER;

/*
 * Number of entries belonging to the account whose history
 * is being queried.
 *
 * The endpoint itself always returns at most 50 rows.
 */
const DATASET_SIZES: &[i32] = &[100, 1_000, 10_000, 100_000];

#[tokio::main]
async fn main() -> BenchResult<()> {
  println!("--- TXN Account History Benchmark ---");
  println!("Workers:              {WORKERS}");
  println!("Reads / worker:       {READS_PER_WORKER}");
  println!("Reads / dataset:      {TOTAL_READS}");
  println!("Endpoint page size:   50");

  for &entry_count in DATASET_SIZES {
    run_dataset(entry_count).await?;
  }

  Ok(())
}

async fn run_dataset(entry_count: i32) -> BenchResult<()> {
  println!();
  println!("============================================================");
  println!("ACCOUNT HISTORY BENCHMARK");
  println!("============================================================");
  println!("Entries on target:    {entry_count}");
  println!("Concurrent workers:   {WORKERS}");
  println!("Total reads:          {TOTAL_READS}");
  println!("Returned rows max:    50");

  // -------------------------------------------------------------------------
  // Setup
  // -------------------------------------------------------------------------

  let bench = BenchClient::new().await?;
  let pool = benchmark_db_pool().await?;

  let equity = bench
    .create_account(&format!("history-{entry_count}-equity"), "Equity")
    .await?;

  let asset = bench
    .create_account(&format!("history-{entry_count}-asset"), "Asset")
    .await?;

  println!("Asset account:        {asset}");
  println!("Equity account:       {equity}");

  println!("--- Seeding {entry_count} transactions ---");

  seed_history(&pool, bench.user_id, asset, equity, entry_count).await?;

  // -------------------------------------------------------------------------
  // Validate fixture
  // -------------------------------------------------------------------------

  let target_entry_count = count_account_entries(&pool, asset).await?;

  if target_entry_count != i64::from(entry_count) {
    return Err(
      format!(
        "fixture entry count mismatch: expected {}, got {}",
        entry_count, target_entry_count
      )
      .into(),
    );
  }

  let balance = db_balance(&pool, asset).await?;

  let expected_balance = Decimal::from(entry_count);

  if balance != expected_balance {
    return Err(
      format!("fixture balance mismatch: expected {expected_balance}, got {balance}").into(),
    );
  }

  let ledger_sum = verify_user_ledger(&pool, bench.user_id).await?;

  if ledger_sum != Decimal::ZERO {
    return Err(format!("fixture violates zero-sum invariant: {ledger_sum}").into());
  }

  println!("Target entries:       {target_entry_count}");
  println!("Target balance:       {balance}");
  println!("Ledger sum:           {ledger_sum}");

  /*
   * Because this benchmark is specifically about the LIMIT 50
   * history endpoint, verify that the database contains more than
   * enough rows to exercise the limit.
   */
  if entry_count < 50 {
    return Err("history benchmark dataset must contain at least 50 entries".into());
  }

  // -------------------------------------------------------------------------
  // Warmup
  // -------------------------------------------------------------------------

  println!("--- Warmup ---");

  for _ in 0..32 {
    let response = bench.get_activity_raw(asset).await?;

    if response.status != StatusCode::OK {
      return Err(format!("warmup returned {}", response.status).into());
    }
  }

  // -------------------------------------------------------------------------
  // Measured workload
  // -------------------------------------------------------------------------

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
       * All workers must be ready before any measured
       * history request is issued.
       */
      barrier.wait().await;

      for _ in 0..READS_PER_WORKER {
        let result = bench.get_activity_raw(asset).await;

        stats.record(result);
      }

      stats
    });
  }

  /*
   * Start timing immediately before releasing the workers.
   */
  let started = Instant::now();

  barrier.wait().await;

  let mut combined = WorkerStats::new();

  while let Some(result) = tasks.join_next().await {
    let stats = result?;

    combined.merge(stats);
  }

  let elapsed = started.elapsed();

  print_results_for_status(entry_count, &combined, elapsed);

  // -------------------------------------------------------------------------
  // Response correctness
  // -------------------------------------------------------------------------

  let successful = combined.successful_status(StatusCode::OK);

  if successful != TOTAL_READS as u64 {
    return Err(format!("expected {TOTAL_READS} HTTP 200 responses, got {successful}").into());
  }

  if combined.client_errors() != 0 {
    return Err(
      format!(
        "history benchmark produced {} 4xx responses",
        combined.client_errors()
      )
      .into(),
    );
  }

  if combined.server_errors() != 0 {
    return Err(
      format!(
        "history benchmark produced {} 5xx responses",
        combined.server_errors()
      )
      .into(),
    );
  }

  if combined.transport_errors != 0 {
    return Err(
      format!(
        "history benchmark produced {} transport errors",
        combined.transport_errors
      )
      .into(),
    );
  }

  /*
   * Now perform one unmeasured request where we actually parse
   * the JSON response.
   *
   * This verifies that the endpoint really returns the latest
   * 50 records rather than merely returning HTTP 200.
   */
  let validation = fetch_history(&bench, asset).await?;

  validate_history_response(&pool, asset, entry_count, validation).await?;

  /*
   * Final ledger invariant.
   */
  let final_sum = verify_user_ledger(&pool, bench.user_id).await?;

  if final_sum != Decimal::ZERO {
    return Err(format!("final ledger invariant failed: {final_sum}").into());
  }

  println!();
  println!("Correctness");
  println!("  HTTP 200 responses: {successful}/{TOTAL_READS}");
  println!("  Expected history size: 50");
  println!("  Ledger sum:         {final_sum}");
  println!("  Result:              PASS");

  Ok(())
}

// ============================================================================
// Fixture
// ============================================================================

async fn seed_history(
  pool: &sqlx::PgPool,
  user_id: i32,
  asset_id: i32,
  equity_id: i32,
  count: i32,
) -> BenchResult<()> {
  let mut tx = pool.begin().await?;

  /*
   * Every generated transaction is:
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
   * request_hash is a deterministic 32-byte value. The actual
   * request fingerprint does not matter because these rows are
   * fixture data and are never looked up through the idempotency
   * endpoint.
   */
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
				'History benchmark seed',
				'history-bench-' || g,
				decode(repeat('00', 32), 'hex')
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

async fn count_account_entries(pool: &sqlx::PgPool, account_id: i32) -> BenchResult<i64> {
  let count = sqlx::query_scalar::<_, i64>(
    r#"
			SELECT COUNT(*)
			FROM entries
			WHERE account_id = $1
			"#,
  )
  .bind(account_id)
  .fetch_one(pool)
  .await?;

  Ok(count)
}

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

// ============================================================================
// HTTP validation
// ============================================================================

async fn fetch_history(bench: &BenchClient, account_id: i32) -> BenchResult<(StatusCode, Value)> {
  let response = bench
    .client
    .get(format!(
      "{}/accounts/{account_id}/transactions",
      bench.base_url
    ))
    .bearer_auth(&bench.token)
    .send()
    .await?;

  let status = response.status();

  let body = response.json::<Value>().await?;

  Ok((status, body))
}

async fn validate_history_response(
  pool: &sqlx::PgPool,
  account_id: i32,
  entry_count: i32,
  (status, body): (StatusCode, Value),
) -> BenchResult<()> {
  if status != StatusCode::OK {
    return Err(format!("history validation request returned {status}").into());
  }

  let rows = body
    .as_array()
    .ok_or("history response is not a JSON array")?;

  /*
   * For all our datasets:
   *
   *     entry_count >= 50
   *
   * so the endpoint should return exactly 50 rows.
   */
  if rows.len() != 50 {
    return Err(format!("expected exactly 50 history rows, got {}", rows.len()).into());
  }

  /*
   * Every row should represent the target Asset account's
   * +1.00 posting.
   */
  for (index, row) in rows.iter().enumerate() {
    let amount = row["amount"]
      .as_str()
      .ok_or_else(|| format!("history row {index} has no string amount"))?;

    if amount != "1.00" && amount != "1" {
      return Err(format!("history row {index} has unexpected amount {amount}").into());
    }

    let description = row["description"]
      .as_str()
      .ok_or_else(|| format!("history row {index} has no description"))?;

    if description != "History benchmark seed" {
      return Err(format!("history row {index} has unexpected description {description:?}").into());
    }
  }

  /*
   * Verify that the first returned transaction is actually the
   * newest entry for this account.
   *
   * The real endpoint orders by:
   *
   *     e.id DESC
   *
   * so the first result should correspond to the greatest entry
   * ID for this account.
   */
  let expected_transaction_id: i32 = sqlx::query_scalar(
    r#"
			SELECT transaction_id
			FROM entries
			WHERE account_id = $1
			ORDER BY id DESC
			LIMIT 1
			"#,
  )
  .bind(account_id)
  .fetch_one(pool)
  .await?;

  let returned_transaction_id = rows[0]["transaction_id"]
    .as_i64()
    .ok_or("first history row has no transaction_id")? as i32;

  if returned_transaction_id != expected_transaction_id {
    return Err(
      format!(
        "history ordering mismatch: expected latest transaction {}, got {}",
        expected_transaction_id, returned_transaction_id
      )
      .into(),
    );
  }

  /*
   * Make sure the dataset actually contains the intended
   * amount of history.
   */
  let actual_count = count_account_entries(
    // We need a pool here, but the caller already gave us one.
    // This check is therefore performed using the same query
    // path as the fixture validation.
    pool, account_id,
  )
  .await?;

  if actual_count != i64::from(entry_count) {
    return Err(
      format!(
        "history count mismatch: expected {}, got {}",
        entry_count, actual_count
      )
      .into(),
    );
  }

  Ok(())
}

// ============================================================================
// Output
// ============================================================================

fn print_results_for_status(entry_count: i32, stats: &WorkerStats, elapsed: Duration) {
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
