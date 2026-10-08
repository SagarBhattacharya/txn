# txn

![CI](https://github.com/SagarBhattacharya/txn/actions/workflows/ci.yml/badge.svg)
![Release](https://img.shields.io/github/v/release/SagarBhattacharya/txn)

**A correctness-first double-entry ledger API built with Rust and PostgreSQL.**

`txn` is an experimental financial ledger focused on the engineering problems behind transactional systems:

- double-entry accounting
- immutable financial records
- concurrency control
- idempotent mutations
- safe reversals
- derived balances
- PostgreSQL locking

![txn dashboard](docs/screenshots/dashboard.png)

## Run

### Requirements

- Docker or Podman
- Compose

Start the full stack:

```bash
podman compose up -d --build
```

Open the web UI:

```text
http://localhost:8000
```

Check readiness:

```bash
curl http://localhost:8000/health/ready
```

Stop:

```bash
podman compose down
```

## Try it

Register a user:

```bash
TOKEN=$(
  curl -s http://localhost:8000/users/register \
    -H 'Content-Type: application/json' \
    -d '{
      "username": "alice",
      "password": "password1234"
    }' |
  jq -r '.token'
)
```

Create two accounts:

```bash
EQUITY_ID=$(
  curl -s http://localhost:8000/accounts \
    -H "Authorization: Bearer $TOKEN" \
    -H 'Content-Type: application/json' \
    -d '{
      "name": "Opening Equity",
      "account_type": "Equity"
    }' |
  jq -r '.id'
)

ASSET_ID=$(
  curl -s http://localhost:8000/accounts \
    -H "Authorization: Bearer $TOKEN" \
    -H 'Content-Type: application/json' \
    -d '{
      "name": "Cash",
      "account_type": "Asset"
    }' |
  jq -r '.id'
)
```

Fund the asset account:

```bash
curl -X POST http://localhost:8000/transactions \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'Idempotency-Key: opening-funding-001' \
  -d "{
    \"source_account_id\": $EQUITY_ID,
    \"destination_account_id\": $ASSET_ID,
    \"amount\": \"1000.00\",
    \"description\": \"Opening funding\"
  }"
```

Check the balance:

```bash
curl -s \
  "http://localhost:8000/accounts/$ASSET_ID/balance" \
  -H "Authorization: Bearer $TOKEN"
```

## API

Authenticated endpoints use:

```text
Authorization: Bearer <token>
```

### Users

| Method | Endpoint          | Description                    |
|--------|-------------------|--------------------------------|
| `POST` | `/users/register` | Register and receive a JWT     |
| `POST` | `/users/login`    | Authenticate and receive a JWT |

### Accounts

| Method | Endpoint                     | Description               |
|--------|------------------------------|---------------------------|
| `POST` | `/accounts`                  | Create an account         |
| `GET`  | `/accounts`                  | List accounts             |
| `GET`  | `/accounts/:id`              | Get an account            |
| `GET`  | `/accounts/:id/balance`      | Calculate current balance |
| `GET`  | `/accounts/:id/transactions` | Get account history       |

### Transactions

| Method | Endpoint                    | Description           |
|--------|-----------------------------|-----------------------|
| `POST` | `/transactions`             | Create a transfer     |
| `POST` | `/transactions/:id/reverse` | Reverse a transaction |

Financial mutations require an `Idempotency-Key`.

## Design

PostgreSQL is the consistency boundary for financial operations.

Every transaction contains balanced ledger entries:

```text
Transaction
└── Entries
    ├── account
    └── amount
```

The core rules are simple:

- every transaction must sum to zero
- entries and transactions are immutable
- balances are derived from ledger entries
- reversals create new inverse transactions
- concurrent mutations lock affected accounts in a deterministic order
- repeated requests with the same idempotency key are serialized and replayed safely

The implementation uses PostgreSQL transactions, advisory locks, `SELECT ... FOR UPDATE`, database constraints, and immutable ledger records.

See [`docs/ARCH.md`](docs/ARCH.md) for the full data model, transaction flows, locking strategy, invariants, rejected alternatives, and scaling considerations.

## Container Image

The API image is published to GitHub Container Registry:

```text
ghcr.io/sagarbhattacharya/txn:v0.1.0
```

The image runs the `txn` API server and requires a PostgreSQL database plus the required environment variables.

For local development, the Compose setup is the easiest way to run the complete stack.

## Benchmarks

`txn` includes a dedicated HTTP benchmark suite covering:

- transfer throughput
- hot-account contention
- idempotency replay and concurrent races
- reversal throughput and concurrent races
- balance reads as ledger history grows
- account-history reads as ledger history grows

Selected `v0.1.0` results:

| Benchmark              |            Result |
|------------------------|------------------:|
| Transfer               |       1,725 req/s |
| Hot-account contention |         187 req/s |
| Independent reversal   |       1,136 req/s |
| Idempotency replay     |         962 req/s |
| Balance reads          | 5,554 → 272 req/s |
| History reads          | 2,217 → 318 req/s |

All runs preserved the zero-sum ledger invariant and completed without unexpected server or transport errors.

Full methodology and results: [`docs/BENCHES.md`](docs/BENCHES.md)

Run the benchmark environment:

```bash
podman compose --profile bench up -d --build
```

Run a benchmark:

```bash
./scripts/run-bench.sh transfer
```

Available benchmarks:

```text
transfer
contention
idempotency
reversal
reads
history
```

## Repository

```text
src/            Application and domain code
benches/        HTTP benchmark suite
docs/           Architecture and benchmark documentation
scripts/        Development and benchmark scripts
migrations/     PostgreSQL migrations
static/         Web UI
compose.yaml    Development and benchmark environments
Dockerfile      Container image
```

## Status

`txn` is a `v0.1.0` experimental project focused on ledger correctness and transactional behavior, not production banking infrastructure.

Known limitations and future work are documented in [`docs/ARCH.md`](docs/ARCH.md).