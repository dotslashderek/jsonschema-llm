#!/usr/bin/env bash
# generate-fixtures.sh — Regenerate pre-built OAS 3.1 fixtures
#
# Usage: ./scripts/generate-fixtures.sh
#
# Prerequisites:
#   - Rust toolchain (cargo)
#   - jq (for JSON validation)
#
# The source schema must already exist at fixtures/oas31/source/oas31-schema.json.
# This script does NOT fetch from the network.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SOURCE_SCHEMA="$PROJECT_ROOT/fixtures/oas31/source/oas31-schema.json"
OUTPUT_DIR="$PROJECT_ROOT/fixtures/oas31/openai-strict"
CLI="$PROJECT_ROOT/target/release/jsonschema-llm"

# ── Step 1: Verify source schema exists ──────────────────────────────────────

if [[ ! -f "$SOURCE_SCHEMA" ]]; then
  echo "ERROR: Source schema not found at $SOURCE_SCHEMA"
  echo "Download it from: https://spec.openapis.org/oas/3.1/schema/2022-10-07"
  exit 1
fi

echo "✅ Source schema: $SOURCE_SCHEMA"

# ── Step 2: Build the CLI ────────────────────────────────────────────────────

echo "🔧 Building CLI (release)..."
cargo build --release -p jsonschema-llm --manifest-path "$PROJECT_ROOT/Cargo.toml" 2>&1 | tail -3

# ── Step 3: Clean previous output ────────────────────────────────────────────

if [[ -d "$OUTPUT_DIR" ]]; then
  echo "🗑️  Removing previous output: $OUTPUT_DIR"
  rm -rf "$OUTPUT_DIR"
fi

# ── Step 4: Generate fixtures ────────────────────────────────────────────────

echo "⚙️  Generating fixtures..."
STDERR_FILE=$(mktemp)
"$CLI" convert "$SOURCE_SCHEMA" --output-dir "$OUTPUT_DIR" 2>"$STDERR_FILE" || true

# Check for component errors (report but don't fail — some recursive schemas are expected to error)
if grep -q "Component error" "$STDERR_FILE"; then
  echo ""
  echo "⚠️  Component errors (expected for deeply recursive schemas):"
  grep "Component error" "$STDERR_FILE" | while read -r line; do
    echo "  $line"
  done
  echo ""
fi
rm -f "$STDERR_FILE"

# ── Step 5: Validate JSON files ──────────────────────────────────────────────

echo "🔍 Validating generated JSON files..."
TOTAL_FILES=0
INVALID_FILES=0

while IFS= read -r -d '' file; do
  TOTAL_FILES=$((TOTAL_FILES + 1))
  if ! jq . "$file" > /dev/null 2>&1; then
    echo "  ❌ Invalid JSON: $file"
    INVALID_FILES=$((INVALID_FILES + 1))
  fi
done < <(find "$OUTPUT_DIR" -name "*.json" -print0)

if [[ $INVALID_FILES -gt 0 ]]; then
  echo "ERROR: $INVALID_FILES invalid JSON files found"
  exit 1
fi

# ── Step 6: Summary ─────────────────────────────────────────────────────────

COMPONENT_COUNT=$(jq '.components | length' "$OUTPUT_DIR/manifest.json")
echo ""
echo "════════════════════════════════════════════════"
echo "  ✅ Fixture generation complete"
echo "  📦 Components: $COMPONENT_COUNT"
echo "  📄 Total files: $TOTAL_FILES"
echo "  📁 Output: $OUTPUT_DIR"
echo "════════════════════════════════════════════════"
