#!/usr/bin/env bash
# Runs the Cybercab Command Center: the API with a simulated Austin fleet, and the
# Astro front end. Open http://localhost:4321 (add ?mode=wall for the wall display).
#
#   SIM_SPEED=4 DEMAND=1.5 ./run.sh
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
PORT="${PORT:-4000}"
cd "$DIR"

echo "-> Generating the TypeScript SDK from the domains..."
cargo run -q -p cybercab -- --codegen-only

cd frontend
if [ ! -d node_modules ]; then
    echo "-> Installing frontend dependencies..."
    if command -v bun >/dev/null 2>&1; then bun install; else npm install; fi
fi

echo "-> Starting the API and the fleet simulation on :$PORT..."
cd "$DIR"
PORT="$PORT" cargo run -q -p cybercab &
BACKEND_PID=$!
cleanup() {
    kill "$BACKEND_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

for _ in {1..60}; do
    curl -s "http://127.0.0.1:$PORT/health" >/dev/null && break
    sleep 0.5
done

echo "-> Starting the command center on http://localhost:4321..."
cd "$DIR/frontend"
export PUBLIC_API_URL="http://127.0.0.1:$PORT"
if command -v bun >/dev/null 2>&1; then bun run dev; else npm run dev; fi
