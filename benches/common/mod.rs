#![allow(unused)]

use hdrhistogram::Histogram;
use reqwest::{Client, StatusCode};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::collections::BTreeMap;
use std::env;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const DEFAULT_BASE_URL: &str = "http://localhost:8001";

pub const DEFAULT_DATABASE_URL: &str =
  "postgres://ledger_user:ledger_secret@127.0.0.1:5433/ledger_bench";

pub type BenchResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

// ============================================================================
// HTTP CLIENT
// ============================================================================

#[derive(Clone)]
pub struct BenchClient {
  pub client: Client,
  pub base_url: String,
  pub token: String,
  pub user_id: i32,
}

impl BenchClient {
  pub async fn new() -> BenchResult<Self> {
    let base_url = env::var("TXN_BENCH_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());

    let client = Client::builder().pool_max_idle_per_host(64).build()?;

    let username = format!("bench_{}", Uuid::new_v4().simple());

    let password = "password1234";

    let response = client
      .post(format!("{base_url}/users/register"))
      .json(&json!({
        "username": username,
        "password": password,
      }))
      .send()
      .await?;

    let status = response.status();
    let body: Value = response.json().await?;

    if status != StatusCode::CREATED {
      return Err(format!("benchmark user creation failed: {status}: {body}").into());
    }

    let user_id = body["user_id"]
      .as_i64()
      .ok_or("registration response missing user_id")? as i32;

    let token = body["token"]
      .as_str()
      .ok_or("registration response missing token")?
      .to_owned();

    Ok(Self {
      client,
      base_url,
      token,
      user_id,
    })
  }

  pub async fn create_account(&self, name: &str, account_type: &str) -> BenchResult<i32> {
    let response = self
      .client
      .post(format!("{}/accounts", self.base_url))
      .bearer_auth(&self.token)
      .json(&json!({
        "name": name,
        "account_type": account_type,
      }))
      .send()
      .await?;

    let status = response.status();
    let body: Value = response.json().await?;

    if status != StatusCode::CREATED {
      return Err(format!("account creation failed: {status}: {body}").into());
    }

    let id = body["id"].as_i64().ok_or("account response missing id")? as i32;

    Ok(id)
  }

  pub async fn transfer(
    &self,
    source_account_id: i32,
    destination_account_id: i32,
    amount: &str,
    idempotency_key: &str,
    description: &str,
  ) -> Result<BenchResponse, reqwest::Error> {
    let started = Instant::now();

    let response = self
      .client
      .post(format!("{}/transactions", self.base_url))
      .bearer_auth(&self.token)
      .header("idempotency-key", idempotency_key)
      .json(&json!({
        "source_account_id": source_account_id,
        "destination_account_id": destination_account_id,
        "amount": amount,
        "description": description,
      }))
      .send()
      .await;

    match response {
      Ok(response) => {
        let status = response.status();

        /*
         * Consume the body so reqwest can reuse the
         * underlying HTTP connection.
         */
        let body = response.bytes().await?;

        let transaction_id = serde_json::from_slice::<Value>(&body)
          .ok()
          .and_then(|body| body["transaction_id"].as_i64())
          .map(|id| id as i32);

        Ok(BenchResponse {
          status,
          latency: started.elapsed(),
          transaction_id,
        })
      }

      Err(error) => Err(error),
    }
  }

  pub async fn reverse(
    &self,
    transaction_id: i32,
    reason: &str,
    idempotency_key: &str,
  ) -> Result<BenchResponse, reqwest::Error> {
    let started = Instant::now();

    let response = self
      .client
      .post(format!(
        "{}/transactions/{transaction_id}/reverse",
        self.base_url
      ))
      .bearer_auth(&self.token)
      .header("idempotency-key", idempotency_key)
      .json(&json!({
        "reason": reason,
      }))
      .send()
      .await;

    match response {
      Ok(response) => {
        let status = response.status();

        let body = response.bytes().await?;

        let transaction_id = serde_json::from_slice::<Value>(&body)
          .ok()
          .and_then(|body| body["transaction_id"].as_i64())
          .map(|id| id as i32);

        Ok(BenchResponse {
          status,
          latency: started.elapsed(),
          transaction_id,
        })
      }

      Err(error) => Err(error),
    }
  }

  pub async fn get_balance(&self, account_id: i32) -> BenchResult<Decimal> {
    let response = self
      .client
      .get(format!("{}/accounts/{account_id}/balance", self.base_url))
      .bearer_auth(&self.token)
      .send()
      .await?;

    let status = response.status();
    let body: Value = response.json().await?;

    if status != StatusCode::OK {
      return Err(format!("balance request failed: {status}: {body}").into());
    }

    let value = body["balance"]
      .as_str()
      .ok_or("balance response missing string balance")?;

    Ok(value.parse()?)
  }

  /*
   * Same request as get_balance(), but returns the raw HTTP
   * response so a read benchmark can record the HTTP status
   * and latency itself.
   */
  pub async fn get_balance_raw(&self, account_id: i32) -> Result<BenchResponse, reqwest::Error> {
    let started = Instant::now();

    let response = self
      .client
      .get(format!("{}/accounts/{account_id}/balance", self.base_url))
      .bearer_auth(&self.token)
      .send()
      .await;

    match response {
      Ok(response) => {
        let status = response.status();

        let body = response.bytes().await?;

        /*
         * Balance reads do not return a transaction ID,
         * so this remains None.
         *
         * The response body is still consumed to allow
         * connection reuse.
         */
        let _ = body;

        Ok(BenchResponse {
          status,
          latency: started.elapsed(),
          transaction_id: None,
        })
      }

      Err(error) => Err(error),
    }
  }

  pub async fn get_activity_raw(&self, account_id: i32) -> Result<BenchResponse, reqwest::Error> {
    let started = std::time::Instant::now();

    let response = self
      .client
      .get(format!(
        "{}/accounts/{account_id}/transactions",
        self.base_url
      ))
      .bearer_auth(&self.token)
      .send()
      .await;

    match response {
      Ok(response) => {
        let status = response.status();

        /*
         * Consume the response body so Reqwest can reuse
         * the connection.
         *
         * We deliberately do not deserialize the response
         * during the measured workload because JSON parsing
         * on the benchmark client is not part of the server
         * operation we are trying to measure.
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
}

// ============================================================================
// RESPONSE
// ============================================================================

#[derive(Debug)]
pub struct BenchResponse {
  pub status: StatusCode,
  pub latency: Duration,
  pub transaction_id: Option<i32>,
}

// ============================================================================
// STATISTICS
// ============================================================================

pub struct WorkerStats {
  pub histogram: Histogram<u64>,
  pub statuses: BTreeMap<u16, u64>,
  pub transport_errors: u64,
}

impl WorkerStats {
  pub fn new() -> Self {
    Self {
      histogram: Histogram::new_with_bounds(1, 60_000_000, 3).expect("valid HDR histogram"),

      statuses: BTreeMap::new(),

      transport_errors: 0,
    }
  }

  pub fn record(&mut self, result: Result<BenchResponse, reqwest::Error>) {
    match result {
      Ok(response) => {
        *self.statuses.entry(response.status.as_u16()).or_default() += 1;

        let micros = response.latency.as_micros() as u64;

        self
          .histogram
          .record(micros.max(1))
          .expect("latency must fit histogram");
      }

      Err(_) => {
        self.transport_errors += 1;
      }
    }
  }

  pub fn merge(&mut self, other: WorkerStats) {
    self
      .histogram
      .add(&other.histogram)
      .expect("histograms must have compatible ranges");

    for (status, count) in other.statuses {
      *self.statuses.entry(status).or_default() += count;
    }

    self.transport_errors += other.transport_errors;
  }

  pub fn total_http_responses(&self) -> u64 {
    self.statuses.values().sum()
  }

  pub fn total_requests(&self) -> u64 {
    self.total_http_responses() + self.transport_errors
  }

  pub fn successful(&self) -> u64 {
    self.successful_status(StatusCode::CREATED)
  }

  pub fn successful_status(&self, status: StatusCode) -> u64 {
    self.statuses.get(&status.as_u16()).copied().unwrap_or(0)
  }

  pub fn client_errors(&self) -> u64 {
    self
      .statuses
      .iter()
      .filter(|(status, _)| (400..500).contains(*status))
      .map(|(_, count)| *count)
      .sum()
  }

  pub fn server_errors(&self) -> u64 {
    self
      .statuses
      .iter()
      .filter(|(status, _)| (500..600).contains(*status))
      .map(|(_, count)| *count)
      .sum()
  }
}

// ============================================================================
// RESULT PRINTING
// ============================================================================

pub fn print_results(title: &str, stats: &WorkerStats, elapsed: Duration) {
  print_results_for_status(title, stats, elapsed, StatusCode::CREATED);
}

pub fn print_results_for_status(
  title: &str,
  stats: &WorkerStats,
  elapsed: Duration,
  success_status: StatusCode,
) {
  let requests = stats.total_requests();

  let throughput = requests as f64 / elapsed.as_secs_f64();

  let successful = stats.successful_status(success_status);

  println!();
  println!("============================================================");
  println!("{title}");
  println!("============================================================");

  println!();
  println!("Requests");
  println!("  Total:              {requests}");
  println!("  Successful ({}):   {successful}", success_status.as_u16());
  println!("  4xx responses:      {}", stats.client_errors());
  println!("  5xx responses:      {}", stats.server_errors());
  println!("  Transport errors:   {}", stats.transport_errors);

  println!();
  println!("HTTP status distribution");

  for (status, count) in &stats.statuses {
    println!("  {status}:              {count}");
  }

  println!();
  println!("Performance");
  println!("  Total time:         {:.2}s", elapsed.as_secs_f64());
  println!("  Throughput:         {:.2} req/s", throughput);

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

  println!("============================================================");
  println!();
}

// ============================================================================
// DATABASE
// ============================================================================

pub async fn benchmark_db_pool() -> BenchResult<PgPool> {
  let url = env::var("TXN_BENCH_DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());

  let pool = PgPoolOptions::new()
    .max_connections(2)
    .connect(&url)
    .await?;

  Ok(pool)
}

pub async fn verify_user_ledger(pool: &PgPool, user_id: i32) -> BenchResult<Decimal> {
  let sum: Decimal = sqlx::query_scalar(
    r#"
		SELECT COALESCE(SUM(e.amount), 0.00)
		FROM entries e
		JOIN transactions t
		  ON t.id = e.transaction_id
		WHERE t.user_id = $1
		"#,
  )
  .bind(user_id)
  .fetch_one(pool)
  .await?;

  Ok(sum)
}
