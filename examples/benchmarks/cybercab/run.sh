#!/usr/bin/env bash
# Benchmarks the Rust and Elixir Cybercab servers against each other: starts Postgres,
# builds both servers for release, runs the scenarios and writes results/RESULTS.md.
#
#   ./run.sh                          # everything, once: about two minutes
#   ./run.sh --reps 3                 # three times; the report takes medians
#   ./run.sh --full                   # bigger fleets, longer windows: far slower
#   ./run.sh --servers rust --scenarios reads,commands
#   POSTGRES=postgres://u:p@host:5432 ./run.sh   # a Postgres of your own
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIR/../../.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
ELIXIR="$ROOT/examples/elixir/cybercab"
CONTAINER="${PG_CONTAINER:-cybercab-bench-pg}"
PG_PORT="${PG_PORT:-55434}"
if [ -n "${POSTGRES:-}" ]; then
    echo "-> Using $POSTGRES; it needs databases cybercab_rust and cybercab_elixir."
else
    POSTGRES="postgres://postgres:postgres@127.0.0.1:$PG_PORT"
    if ! docker ps --format '{{.Ports}}' | grep -q ":$PG_PORT->"; then
        echo "-> Starting Postgres 16 on :$PG_PORT..."
        docker run -d --rm --name "$CONTAINER" -e POSTGRES_PASSWORD=postgres \
            -p "$PG_PORT:5432" postgres:16 >/dev/null
        until docker exec "$CONTAINER" pg_isready -U postgres >/dev/null 2>&1; do sleep 0.5; done
    fi
    running="$(docker ps --format '{{.Names}} {{.Ports}}' | grep ":$PG_PORT->" | cut -d' ' -f1 | head -1)"
    for db in cybercab_rust cybercab_elixir; do
        docker exec "$running" psql -U postgres -tc "SELECT 1 FROM pg_database WHERE datname = '$db'" \
            | grep -q 1 || docker exec "$running" psql -U postgres -qc "CREATE DATABASE $db"
    done
fi

echo "-> Building the Rust server and the driver (release)..."
(cd "$ROOT" && cargo build -q --release -p cybercab -p cybercab-bench)

echo "-> Building the Elixir server (MIX_ENV=prod release)..."
(cd "$ELIXIR" && MIX_ENV=prod mix deps.get >/dev/null && MIX_ENV=prod mix release --overwrite --quiet)

echo "-> Running the scenarios..."
"$TARGET/release/cybercab-bench" run \
    --rust-bin "$TARGET/release/cybercab" \
    --elixir-release "$ELIXIR/_build/prod/rel/cybercab" \
    --postgres "$POSTGRES" \
    --out "$DIR/results" \
    "$@"
