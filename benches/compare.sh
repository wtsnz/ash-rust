#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

echo "=========================================================="
echo "          Ash Benchmark Suite: Rust vs. Elixir           "
echo "=========================================================="
echo ""

echo "[1/4] Running ash-rust core release benchmark (helpdesk)..."
cargo run --release -p helpdesk --example bench

echo ""
echo "[2/4] Running ash-graphql real-world API release benchmark..."
cargo run --release -p ash-graphql --example bench_graphql --features axum

echo ""
echo "[3/4] Running canonical Ash (Elixir + ETS) core benchmark..."
if command -v mise >/dev/null 2>&1; then
    mise exec erlang@27.1.1 -- elixir benches/ash_elixir_bench.exs
elif command -v elixir >/dev/null 2>&1; then
    elixir benches/ash_elixir_bench.exs
else
    echo "Warning: Elixir is not installed or not in PATH. Skipping Elixir core suite."
fi

echo ""
echo "[4/4] Running Ash Elixir + Absinthe GraphQL benchmark..."
if command -v mise >/dev/null 2>&1; then
    mise exec erlang@27.1.1 -- elixir benches/ash_graphql_elixir_bench.exs
elif command -v elixir >/dev/null 2>&1; then
    elixir benches/ash_graphql_elixir_bench.exs
else
    echo "Warning: Elixir is not installed or not in PATH. Skipping Elixir GraphQL suite."
fi

if [[ "${1:-}" == "--postgres" || "${1:-}" == "--all" || "${INCLUDE_POSTGRES:-0}" == "1" ]]; then
    echo ""
    echo "[5/5] Running PostgreSQL benchmark suite (Docker + ash-postgres + AshPostgres)..."
    ./benches/bench_postgres.sh
fi

echo ""
echo "=========================================================="
echo "Benchmark run complete."
echo "To run statistical regression analysis with Criterion:"
echo "  cargo bench -p helpdesk"
echo "  cargo bench -p ash-graphql --bench graphql_bench --features axum"
echo "  cargo bench -p ash-postgres"
echo ""
echo "To run PostgreSQL benchmarks against Docker:"
echo "  ./benches/bench_postgres.sh"
echo "=========================================================="
