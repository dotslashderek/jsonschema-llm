#!/usr/bin/env bash
# Run WASI wrapper tests in Docker for all (or selected) languages.
#
# Usage:
#   ./scripts/test-wrappers.sh           # all 6 languages
#   ./scripts/test-wrappers.sh go python  # specific languages
#
# Prerequisites:
#   1. WASI binary built: cargo build --target wasm32-wasip1 --release -p json-schema-llm-wasi
#   2. Docker and Docker Compose installed

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

WASM_BINARY="$PROJECT_ROOT/target/wasm32-wasip1/release/json_schema_llm_wasi.wasm"

# --- Pre-flight checks ---
if ! docker compose version &>/dev/null && ! docker-compose version &>/dev/null; then
  echo "❌ docker compose not found. Install Docker Desktop or docker-compose."
  exit 1
fi

if [ ! -f "$WASM_BINARY" ]; then
  echo "⚠️  WASI binary not found at: $WASM_BINARY"
  echo "   Building it now..."
  cargo build --target wasm32-wasip1 --release -p json-schema-llm-wasi
fi

# --- Determine which services to test ---
SERVICES=("$@")
if [ ${#SERVICES[@]} -eq 0 ]; then
  SERVICES=(go python ts java ruby dotnet)
fi

echo "🐳 Running wrapper tests for: ${SERVICES[*]}"
echo ""

FAILED=()
for svc in "${SERVICES[@]}"; do
  echo "━━━ Testing: $svc ━━━"
  if docker compose -f "$PROJECT_ROOT/docker-compose.test.yml" up --build --abort-on-container-exit "$svc" 2>&1; then
    echo "✅ $svc passed"
  else
    echo "❌ $svc failed"
    FAILED+=("$svc")
  fi
  docker compose -f "$PROJECT_ROOT/docker-compose.test.yml" down --remove-orphans 2>/dev/null
  echo ""
done

# --- Summary ---
echo "━━━ Summary ━━━"
echo "Total: ${#SERVICES[@]}, Passed: $((${#SERVICES[@]} - ${#FAILED[@]})), Failed: ${#FAILED[@]}"

if [ ${#FAILED[@]} -gt 0 ]; then
  echo "❌ Failed services: ${FAILED[*]}"
  exit 1
fi

echo "✅ All wrapper tests passed!"
