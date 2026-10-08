#!/usr/bin/env bash
set -euo pipefail

echo "Resetting benchmark database..."

podman exec txn-pg-bench \
  psql \
    -U ledger_user \
    -d ledger_bench \
    -v ON_ERROR_STOP=1 \
    -c '
      TRUNCATE
        entries,
        transactions,
        accounts,
        users
      RESTART IDENTITY
      CASCADE;
    '

echo "Benchmark database reset."