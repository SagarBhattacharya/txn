mod common;

use common::{
  BenchClient, BenchResult, WorkerStats, benchmark_db_pool, print_results, verify_user_ledger,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Barrier;
use tokio::task::JoinSet;

const WORKERS: usize = 16;
const OPERATIONS_PER_WORKER: usize = 625;

const INITIAL_BALANCE: &str = "1000000.00";
const TRANSFER_AMOUNT: &str = "5.00";

#[tokio::main]
async fn main() -> BenchResult<()> {
  let total_operations = WORKERS * OPERATIONS_PER_WORKER;

  println!("--- TXN Transfer Throughput Benchmark ---");
  println!("Workers:              {WORKERS}");
  println!("Operations / worker:  {OPERATIONS_PER_WORKER}");
  println!("Total operations:     {total_operations}");

  let bench = BenchClient::new().await?;

  println!("--- Setting up isolated account pairs ---");

  let equity = bench.create_account("bench-equity", "Equity").await?;

  let mut pairs = Vec::with_capacity(WORKERS);

  for worker_id in 0..WORKERS {
    let source = bench
      .create_account(&format!("bench-source-{worker_id}"), "Asset")
      .await?;

    let destination = bench
      .create_account(&format!("bench-destination-{worker_id}"), "Asset")
      .await?;

    /*
     * Fund this worker's source account.
     *
     * This happens before timing starts and therefore does
     * not contribute to the benchmark result.
     */
    let funding = bench
      .transfer(
        equity,
        source,
        INITIAL_BALANCE,
        &format!("setup-funding-{worker_id}"),
        "Benchmark funding",
      )
      .await?;

    if funding.status != axum::http::StatusCode::CREATED {
      return Err(
        format!(
          "funding request failed for worker {worker_id}: {}",
          funding.status
        )
        .into(),
      );
    }

    pairs.push((source, destination));
  }

  /*
   * Warm up one request per worker to establish HTTP/TCP
   * connections before the measured region.
   */
  println!("--- Warmup ---");

  for (worker_id, &(source, destination)) in pairs.iter().enumerate() {
    let warmup = bench
      .transfer(
        source,
        destination,
        "1.00",
        &format!("warmup-{worker_id}"),
        "Benchmark warmup",
      )
      .await?;

    if warmup.status != axum::http::StatusCode::CREATED {
      return Err(
        format!(
          "warmup request failed for worker {worker_id}: {}",
          warmup.status
        )
        .into(),
      );
    }
  }

  println!("--- Starting measured workload ---");

  let bench = Arc::new(bench);
  let pairs = Arc::new(pairs);

  let barrier = Arc::new(Barrier::new(WORKERS + 1));
  let mut join_set = JoinSet::new();

  for worker_id in 0..WORKERS {
    let bench = Arc::clone(&bench);
    let pairs = Arc::clone(&pairs);
    let barrier = Arc::clone(&barrier);

    join_set.spawn(async move {
      let mut stats = WorkerStats::new();

      let (source, destination) = pairs[worker_id];

      // Wait for all workers to be ready.
      barrier.wait().await;

      for operation in 0..OPERATIONS_PER_WORKER {
        let key = format!("transfer-{worker_id}-{operation}");

        let result = bench
          .transfer(
            source,
            destination,
            TRANSFER_AMOUNT,
            &key,
            "Baseline transfer",
          )
          .await;

        stats.record(result);
      }

      stats
    });
  }

  /*
   * Start the wall clock immediately before releasing the
   * workers. The setup and warmup are outside the measured
   * interval.
   */
  let started = Instant::now();
  barrier.wait().await;

  let mut combined = WorkerStats::new();

  while let Some(result) = join_set.join_next().await {
    let stats = result?;
    combined.merge(stats);
  }

  let elapsed = started.elapsed();

  print_results("TXN TRANSFER THROUGHPUT", &combined, elapsed);

  /*
   * Every worker performed:
   *
   *   1 × warmup @ 1.00
   *   625 × measured transfers @ 5.00
   *
   * Therefore:
   *
   *   source
   *     = 1,000,000.00
   *     - 1.00
   *     - (625 × 5.00)
   *     = 996,874.00
   *
   *   destination
   *     = 1.00
   *     + (625 × 5.00)
   *     = 3,126.00
   */

  println!("--- Verifying balances ---");

  for (worker_id, &(source, destination)) in pairs.iter().enumerate() {
    let source_balance = bench.get_balance(source).await?;
    let destination_balance = bench.get_balance(destination).await?;

    let expected_source = rust_decimal::dec!(1000000.00)
      - rust_decimal::dec!(1.00)
      - rust_decimal::dec!(5.00) * rust_decimal::Decimal::from(OPERATIONS_PER_WORKER as u32);

    let expected_destination = rust_decimal::dec!(1.00)
      + rust_decimal::dec!(5.00) * rust_decimal::Decimal::from(OPERATIONS_PER_WORKER as u32);

    if source_balance != expected_source {
      return Err(
        format!(
          "worker {worker_id}: unexpected source balance \
         {source_balance}, expected {expected_source}"
        )
        .into(),
      );
    }

    if destination_balance != expected_destination {
      return Err(
        format!(
          "worker {worker_id}: unexpected destination balance \
         {destination_balance}, expected {expected_destination}"
        )
        .into(),
      );
    }
  }

  let db = benchmark_db_pool().await?;
  let user_sum = verify_user_ledger(&db, bench.user_id).await?;

  println!("--- Correctness ---");
  println!("Benchmark user ledger sum: {user_sum}");

  if user_sum != rust_decimal::dec!(0.00) {
    return Err(format!("ledger invariant failed: benchmark user sum = {user_sum}").into());
  }

  /*
   * For this benchmark every operation should return 201.
   * Unlike the old contention benchmark, no non-201 result
   * is silently ignored.
   */
  if combined.successful() != total_operations as u64
    || combined.client_errors() != 0
    || combined.server_errors() != 0
    || combined.transport_errors != 0
  {
    return Err("benchmark produced unexpected responses; see status distribution above".into());
  }

  println!("All {total_operations} measured transfers succeeded.");
  println!("Ledger invariant: PASS");

  Ok(())
}
