mod common;

use common::{
  BenchClient, BenchResult, WorkerStats, benchmark_db_pool, print_results, verify_user_ledger,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Barrier;
use tokio::task::JoinSet;

const WORKERS: usize = 50;
const OPERATIONS_PER_WORKER: usize = 100;

const INITIAL_BALANCE: &str = "500000.00";
const TRANSFER_AMOUNT: &str = "5.00";

#[tokio::main]
async fn main() -> BenchResult<()> {
  let total_operations = WORKERS * OPERATIONS_PER_WORKER;

  println!("--- TXN Hot-Account Contention Benchmark ---");
  println!("Workers:              {WORKERS}");
  println!("Operations / worker:  {OPERATIONS_PER_WORKER}");
  println!("Total operations:     {total_operations}");

  let bench = BenchClient::new().await?;

  println!("--- Setting up hot accounts ---");

  let equity = bench.create_account("contention-equity", "Equity").await?;

  let account_a = bench.create_account("contention-a", "Asset").await?;

  let account_b = bench.create_account("contention-b", "Asset").await?;

  /*
   * IMPORTANT:
   *
   * Both hot accounts receive liquidity.
   *
   * The old benchmark only funded A:
   *
   *     A = 1,000,000
   *     B = 0
   *
   * but then immediately issued B -> A requests.
   *
   * Those requests could legitimately return 422
   * InsufficientFunds and were silently omitted from the
   * benchmark output.
   *
   * With both accounts funded, a 422 is genuinely unexpected.
   */

  let funding_a = bench
    .transfer(
      equity,
      account_a,
      INITIAL_BALANCE,
      "setup-funding-a",
      "Contention benchmark funding",
    )
    .await?;

  if funding_a.status != axum::http::StatusCode::CREATED {
    return Err(format!("failed to fund account A: {}", funding_a.status).into());
  }

  let funding_b = bench
    .transfer(
      equity,
      account_b,
      INITIAL_BALANCE,
      "setup-funding-b",
      "Contention benchmark funding",
    )
    .await?;

  if funding_b.status != axum::http::StatusCode::CREATED {
    return Err(format!("failed to fund account B: {}", funding_b.status).into());
  }

  println!("Account A: {account_a}");
  println!("Account B: {account_b}");

  println!("--- Warmup ---");

  /*
   * One warmup transfer per worker.
   *
   * Half of the workers go A -> B and half go B -> A,
   * so the hot accounts remain balanced overall.
   */
  for worker_id in 0..WORKERS {
    let (source, destination) = if worker_id % 2 == 0 {
      (account_a, account_b)
    } else {
      (account_b, account_a)
    };

    let warmup = bench
      .transfer(
        source,
        destination,
        "1.00",
        &format!("warmup-{worker_id}"),
        "Contention warmup",
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

  let barrier = Arc::new(Barrier::new(WORKERS + 1));
  let mut join_set = JoinSet::new();

  for worker_id in 0..WORKERS {
    let bench = Arc::clone(&bench);
    let barrier = Arc::clone(&barrier);

    join_set.spawn(async move {
      let mut stats = WorkerStats::new();

      barrier.wait().await;

      for operation in 0..OPERATIONS_PER_WORKER {
        /*
         * Alternate direction between operations.
         *
         * Every worker performs 50 transfers in each
         * direction because OPERATIONS_PER_WORKER = 100.
         */
        let (source, destination) = if (worker_id + operation) % 2 == 0 {
          (account_a, account_b)
        } else {
          (account_b, account_a)
        };

        let key = format!("contention-{worker_id}-{operation}");

        let result = bench
          .transfer(
            source,
            destination,
            TRANSFER_AMOUNT,
            &key,
            "Hot-account contention transfer",
          )
          .await;

        stats.record(result);
      }

      stats
    });
  }

  let started = Instant::now();
  barrier.wait().await;

  println!(
    "Blasting {WORKERS} concurrent workers \
		 ({total_operations} total operations)..."
  );

  let mut combined = WorkerStats::new();

  while let Some(result) = join_set.join_next().await {
    let stats = result?;
    combined.merge(stats);
  }

  let elapsed = started.elapsed();

  print_results("TXN HOT-ACCOUNT CONTENTION", &combined, elapsed);

  /*
   * Every worker executes 50 A -> B and 50 B -> A.
   *
   * Across all workers:
   *
   *     A net change = 0
   *     B net change = 0
   *
   * Therefore both accounts should still contain exactly
   * their original 500,000.00 balance after the measured run.
   */

  println!("--- Verifying balances ---");

  let balance_a = bench.get_balance(account_a).await?;
  let balance_b = bench.get_balance(account_b).await?;

  println!("Account A balance: {balance_a}");
  println!("Account B balance: {balance_b}");

  if balance_a != rust_decimal::dec!(500000.00) {
    return Err(format!("account A invariant failed: expected 500000.00, got {balance_a}").into());
  }

  if balance_b != rust_decimal::dec!(500000.00) {
    return Err(format!("account B invariant failed: expected 500000.00, got {balance_b}").into());
  }

  let db = benchmark_db_pool().await?;
  let user_sum = verify_user_ledger(&db, bench.user_id).await?;

  println!("Benchmark user ledger sum: {user_sum}");

  if user_sum != rust_decimal::dec!(0.00) {
    return Err(format!("ledger invariant failed: benchmark user sum = {user_sum}").into());
  }

  /*
   * For this benchmark, all 5,000 measured requests should
   * return 201 Created.
   *
   * Any 422, 4xx, 5xx, or transport error is an actual
   * benchmark failure and is now visible.
   */
  if combined.successful() != total_operations as u64
    || combined.client_errors() != 0
    || combined.server_errors() != 0
    || combined.transport_errors != 0
  {
    return Err("benchmark produced unexpected responses; see status distribution above".into());
  }

  println!("All {total_operations} measured transfers succeeded.");
  println!("No 4xx responses.");
  println!("No 5xx responses.");
  println!("No transport errors.");
  println!("Ledger invariant: PASS");

  Ok(())
}
