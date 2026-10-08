#!/usr/bin/env bash
set -euo pipefail

BENCHMARK="${1:?Usage: ./scripts/run-bench.sh <benchmark>}"

./scripts/reset-bench-db.sh

echo
echo "Running benchmark: ${BENCHMARK}"
echo

cargo bench --bench "${BENCHMARK}"