# Benchmarks

## Environment

All benchmarks were run on the following system:

| Component           | Details               |
|---------------------|-----------------------|
| OS                  | Fedora Linux 44       |
| Kernel              | 7.2.8-200.fc44.x86_64 |
| CPU                 | AMD Ryzen 5 5500U     |
| CPU topology        | 6 cores / 12 threads  |
| Memory              | 8 GB RAM              |
| Swap                | 8 GB                  |
| Rust                | 1.99.0                |
| Cargo               | 1.99.0                |
| Podman              | 5.8.7                 |
| PostgreSQL          | 18-alpine             |
| Application DB pool | 10 connections        |

The benchmark application runs in the `txn-bench` container. Benchmark workers are configured separately for each benchmark and are listed with the corresponding results.

## Methodology

The benchmarks measure the running HTTP service rather than individual functions.

The benchmark flow is:

```text
cargo bench
    ↓
HTTP requests
    ↓
txn-bench container
    ↓
PostgreSQL
```

Each benchmark is run against a dedicated benchmark database. The database is reset before each benchmark, so previous benchmark data does not affect the results.

All benchmark binaries are built in the optimized `bench` profile.

Before each measured workload, the benchmark performs its setup and a short warmup. Setup and warmup are excluded from the reported timing.

The measured workload is executed concurrently using the number of workers defined by each benchmark. All HTTP requests use the benchmark client and persistent connections.

For each request, the benchmark records:

- total requests
- HTTP status distribution
- transport errors
- throughput
- p50 latency
- p90 latency
- p95 latency
- p99 latency
- maximum latency

Latency is recorded with HDR Histogram.

After the measured workload, benchmarks perform separate correctness checks. Depending on the benchmark, these include expected HTTP status codes, account balances, transaction counts, reversal counts, and the ledger zero-sum invariant:

```text
SUM(entries.amount) = 0
```

Correctness checks are not included in the reported performance measurements.

The benchmark suite currently contains:

| Benchmark     | Purpose                                                    |
|---------------|------------------------------------------------------------|
| `transfer`    | Baseline transfer throughput                               |
| `contention`  | Concurrent transfers against the same accounts             |
| `idempotency` | Fresh requests, replays, and concurrent same-key requests  |
| `reversal`    | Independent reversals and concurrent same-target reversals |
| `reads`       | Balance-read performance as ledger history grows           |
| `history`     | Account-history performance as ledger history grows        |

## Transfer

The transfer benchmark measures baseline write performance with low account contention.

16 workers each use an independent source/destination account pair. Each worker performs 625 transfers for a total of 10,000 measured requests.

| Metric           |         Result |
|------------------|---------------:|
| Workers          |             16 |
| Total requests   |         10,000 |
| Successful       |         10,000 |
| 4xx              |              0 |
| 5xx              |              0 |
| Transport errors |              0 |
| Throughput       | 1,725.30 req/s |
| p50              |        8.89 ms |
| p90              |       11.65 ms |
| p95              |       12.61 ms |
| p99              |       16.73 ms |
| Max              |       56.32 ms |

Correctness checks passed:

```text
10,000 / 10,000 requests returned 201
benchmark user ledger sum = 0
ledger invariant = PASS
```

## Contention

The contention benchmark measures transfer performance when many concurrent requests operate on the same two accounts.

50 workers perform 100 transfers each, for a total of 5,000 requests. Both accounts are pre-funded, and the transfer direction alternates between them. This creates deliberate contention on the account rows locked by the ledger.

| Metric           |       Result |
|------------------|-------------:|
| Workers          |           50 |
| Total requests   |        5,000 |
| Successful       |        5,000 |
| 4xx              |            0 |
| 5xx              |            0 |
| Transport errors |            0 |
| Throughput       | 187.34 req/s |
| p50              |    263.94 ms |
| p90              |    349.44 ms |
| p95              |    358.14 ms |
| p99              |    365.57 ms |
| Max              |    553.47 ms |

Correctness checks passed:

```text
5,000 / 5,000 requests returned 201
Account A balance = 500000.00
Account B balance = 500000.00
benchmark user ledger sum = 0
ledger invariant = PASS
```

## Idempotency

The idempotency benchmark measures the cost and correctness of duplicate requests.

It contains three cases:

1. A fresh request with a new idempotency key.
2. 10,000 sequential replays of the same request and key.
3. 50 concurrent requests using the same key.

| Metric                           |       Result |
|----------------------------------|-------------:|
| Fresh request                    |      5.04 ms |
| Sequential replays               |       10,000 |
| Replay throughput                | 961.90 req/s |
| Replay p50                       |      1.12 ms |
| Replay p95                       |      1.52 ms |
| Replay p99                       |      1.61 ms |
| Replay max                       |      2.65 ms |
| Concurrent same-key requests     |           50 |
| Concurrent requests returned 201 |           50 |
| Transactions created in race     |            1 |
| Entries created in race          |            2 |

All requests returned successfully:

```text
Sequential replay:       10,000 / 10,000 → 201
Concurrent same-key:          50 / 50 → 201
4xx responses:                   0
5xx responses:                   0
Transport errors:               0
```

The concurrent race created exactly one transaction and two entries. Replaying the same request did not create additional ledger entries.

Correctness checks passed:

```text
Asset balance after replays = 999999.00
Concurrent race transactions created = 1
Concurrent race entries created = 2
benchmark user ledger sum = 0
idempotency invariant = PASS
```

## Reversal

The reversal benchmark measures both normal reversal throughput and concurrent attempts to reverse the same transaction.

### Independent reversals

16 workers reverse 4,000 previously created transactions. Each reversal targets a different transaction, so the measured workload does not intentionally contend on the same transaction.

| Metric           |         Result |
|------------------|---------------:|
| Workers          |             16 |
| Total requests   |          4,000 |
| Successful       |          4,000 |
| 4xx              |              0 |
| 5xx              |              0 |
| Transport errors |              0 |
| Throughput       | 1,135.98 req/s |
| p50              |       13.65 ms |
| p90              |       17.41 ms |
| p95              |       19.28 ms |
| p99              |       22.64 ms |
| Max              |       31.55 ms |

Correctness checks passed:

```text
4,000 / 4,000 requests returned 201
benchmark user ledger sum = 0
reversal correctness = PASS
```

### Same-target race

50 concurrent requests attempted to reverse the same transaction using different idempotency keys.

| Metric               |       Result |
|----------------------|-------------:|
| Concurrent requests  |           50 |
| Successful reversals |            1 |
| 422 responses        |           49 |
| 5xx                  |            0 |
| Transport errors     |            0 |
| Throughput           | 626.81 req/s |
| p50                  |     53.76 ms |
| p95                  |     73.66 ms |
| p99                  |     74.43 ms |
| Max                  |     74.43 ms |

Only one reversal was created. The other 49 requests correctly received `422 Unprocessable Entity`.

```text
Reversal transactions created = 1
Source balance = 1000.00
Destination balance = 0
benchmark user ledger sum = 0
same-target reversal invariant = PASS
```

## Balance Reads

This benchmark measures the cost of calculating an account balance as the number of ledger entries grows.

16 workers perform 10,000 balance reads for each dataset size. The benchmark account contains 100, 1,000, 10,000, or 100,000 entries.

| Entries |     Throughput |      p50 |      p90 |      p95 |       p99 |       Max |
|--------:|---------------:|---------:|---------:|---------:|----------:|----------:|
|     100 | 5,553.54 req/s |  2.80 ms |  3.58 ms |  3.86 ms |   4.44 ms |  10.22 ms |
|   1,000 | 4,918.76 req/s |  3.16 ms |  3.96 ms |  4.26 ms |   4.97 ms |   6.98 ms |
|  10,000 | 1,662.66 req/s |  9.00 ms | 12.98 ms | 14.81 ms |  19.05 ms |  35.01 ms |
| 100,000 |   272.40 req/s | 56.19 ms | 72.89 ms | 85.18 ms | 114.75 ms | 198.01 ms |

All requests returned HTTP 200 with no client errors, server errors, or transport failures.

Correctness checks passed for every dataset:

```text
100 / 10000 reads successful
1000 / 10000 reads successful
10000 / 10000 reads successful
100000 / 10000 reads successful

Final balance matched the expected value.
Benchmark user ledger sum = 0.
```

The results show a significant increase in balance-read latency as account history grows. This reflects the current design of deriving balances by summing ledger entries on each read.

## History

The history benchmark measures account-history read performance as the number of ledger entries grows.

16 workers perform 10,000 history reads for each dataset size. The endpoint returns at most 50 records.

| Entries |     Throughput |      p50 |      p90 |      p95 |      p99 |      Max |
|--------:|---------------:|---------:|---------:|---------:|---------:|---------:|
|     100 | 2,217.04 req/s |  6.88 ms |  8.70 ms |  9.68 ms | 13.69 ms | 24.21 ms |
|   1,000 | 2,776.53 req/s |  4.40 ms | 10.63 ms | 11.97 ms | 15.03 ms | 22.75 ms |
|  10,000 | 2,005.23 req/s |  7.88 ms |  9.42 ms |  9.99 ms | 11.32 ms | 14.62 ms |
| 100,000 |   318.00 req/s | 50.02 ms | 60.19 ms | 62.81 ms | 68.03 ms | 92.42 ms |

All requests returned HTTP 200 with no client errors, server errors, or transport failures. The endpoint returned the expected 50 history records for every dataset.

Correctness checks passed:

```text
10,000 / 10,000 reads successful for every dataset
history response size = 50
benchmark user ledger sum = 0
```

The results show that the current history query becomes significantly more expensive as account history grows, despite the response being limited to 50 records.

## Summary

The benchmark suite covers the main performance and concurrency characteristics of the ledger.

Baseline transfers reached **1,725 req/s** at 16 workers with p99 latency of **16.73 ms**. Under deliberate hot-account contention, throughput dropped to **187 req/s** and p99 latency increased to **365.57 ms**, demonstrating the cost of serializing concurrent operations on the same accounts.

Idempotency behaved as intended. A 10,000-request sequential replay workload achieved **961.90 req/s** with **1.12 ms p50** latency, while a 50-request concurrent same-key race created exactly **one transaction** and **two entries**.

Independent reversals reached **1,135.98 req/s** at 16 workers. A 50-request same-target reversal race created exactly one reversal and returned `422` for the other 49 requests.

Read performance depends strongly on ledger history. Balance reads degraded from **5,553.54 req/s** at 100 entries to **272.40 req/s** at 100,000 entries, reflecting the cost of deriving balances by summing postings on each read.

History reads also degraded at large history sizes. Throughput fell from **2,217.04 req/s** at 100 entries to **318.00 req/s** at 100,000 entries, despite the endpoint returning only 50 records.

All benchmark runs preserved the ledger's zero-sum invariant and completed without unexpected 4xx, 5xx, or transport errors.

These results establish a clear baseline for `v0.1.0` and identify derived reads and high-contention writes as the main areas for future optimization.