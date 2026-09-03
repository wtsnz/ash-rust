#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export PATH="$HOME/.cargo/bin:$PATH"

echo "=========================================================="
echo "          Ash Benchmark Suite: Rust vs. Elixir           "
echo "=========================================================="
echo ""

echo "[1/2] Running ash-rust release benchmark..."
cargo run --release -p helpdesk --example bench

echo ""
echo "[2/2] Running canonical Ash (Elixir + ETS) benchmark..."
if command -v mise >/dev/null 2>&1; then
    mise exec elixir erlang -- elixir benches/ash_elixir_bench.exs
elif command -v elixir >/dev/null 2>&1; then
    elixir benches/ash_elixir_bench.exs
else
    echo "Warning: Elixir is not installed or not in PATH. Skipping Elixir suite."
fi

echo ""
echo "=========================================================="
echo "Benchmark run complete."
echo "To run statistical regression analysis with Criterion:"
echo "  cargo bench -p helpdesk"
echo "=========================================================="
