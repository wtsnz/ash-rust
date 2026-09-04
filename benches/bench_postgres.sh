#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

POSTGRES_PORT="${POSTGRES_PORT:-5433}"
export DATABASE_URL="${DATABASE_URL:-postgres://ash_user:ash_password@localhost:${POSTGRES_PORT}/ash_benchmark}"

echo "=========================================================="
echo "      PostgreSQL Ash Benchmark: ash-rust vs. Ash Elixir   "
echo "=========================================================="
echo "Target PostgreSQL URL: ${DATABASE_URL}"
echo ""

# 1. Ensure Docker Compose PostgreSQL container is running
if command -v docker >/dev/null 2>&1; then
    echo "[+] Ensuring PostgreSQL Docker container is running..."
    docker compose up -d
    
    echo "[+] Waiting for PostgreSQL to be healthy..."
    for i in {1..30}; do
        if docker compose exec -T postgres pg_isready -U ash_user -d ash_benchmark >/dev/null 2>&1; then
            echo "[+] PostgreSQL is ready!"
            break
        fi
        sleep 1
    done
else
    echo "Warning: docker is not found. Attempting to connect to existing DATABASE_URL..."
fi

echo ""
echo "[1/2] Running ash-rust PostgreSQL release benchmark (ash-postgres)..."
cargo run --release -p ash-postgres --example bench_postgres

echo ""
echo "[2/2] Running canonical Ash Elixir PostgreSQL benchmark (ash_postgres + Ecto + Postgrex)..."
if command -v mise >/dev/null 2>&1; then
    mise exec erlang@27.1.1 -- elixir benches/ash_postgres_elixir_bench.exs
elif command -v elixir >/dev/null 2>&1; then
    elixir benches/ash_postgres_elixir_bench.exs
else
    echo "Warning: Elixir is not installed or not in PATH. Skipping Elixir suite."
fi

echo ""
echo "=========================================================="
echo "PostgreSQL benchmark run complete."
echo "To run statistical regression analysis with Criterion:"
echo "  cargo bench -p ash-postgres"
echo "=========================================================="
