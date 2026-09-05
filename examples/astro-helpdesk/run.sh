#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
cd "$DIR"

echo "=========================================================="
echo " 🦀 ash-rust + 🚀 Astro Full-Stack Helpdesk Demo"
echo "=========================================================="

# 1. Generate TypeScript SDK
echo "-> 1. Generating TypeScript SDK from Ash Domain..."
cargo run --bin astro-helpdesk-server -- --codegen-only

# 2. Check frontend dependencies
cd frontend
if [ ! -d "node_modules" ]; then
    echo "-> 2. Installing frontend dependencies..."
    if command -v bun >/dev/null 2>&1; then
        bun install
    else
        npm install
    fi
fi

# 3. Start Rust Backend Server
echo "-> 3. Starting Rust GraphQL Server on port 4000..."
cd "$DIR"
cargo run --bin astro-helpdesk-server &
BACKEND_PID=$!

# Cleanup on exit
cleanup() {
    echo -e "\nShutting down fullstack servers..."
    kill $BACKEND_PID 2>/dev/null || true
    exit 0
}
trap cleanup SIGINT SIGTERM EXIT

# Wait for backend health check
echo "Waiting for backend to be ready..."
for i in {1..30}; do
    if curl -s http://127.0.0.1:4000/health >/dev/null; then
        break
    fi
    sleep 0.5
done

# 4. Start Astro Frontend Dev Server
echo "-> 4. Starting Astro Frontend on http://localhost:4321..."
cd "$DIR/frontend"
if command -v bun >/dev/null 2>&1; then
    bun run dev
else
    npm run dev
fi
