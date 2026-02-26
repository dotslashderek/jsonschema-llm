#!/usr/bin/env bash
# generate-fixtures.sh — Regenerate pre-built fixtures for specification schemas
#
# Usage: ./scripts/generate-fixtures.sh [spec]
#   spec: oas31 | arazzo | all (default: all)
#
# Prerequisites:
#   - Rust toolchain (cargo)
#   - jq (for JSON validation)
#
# Source schemas must already exist in fixtures/<spec>/source/.
# This script does NOT fetch from the network.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CLI="$PROJECT_ROOT/target/release/json-schema-llm"

# ── Spec configuration ──────────────────────────────────────────────────────

spec_source() {
  case "$1" in
    oas31)    echo "$PROJECT_ROOT/fixtures/oas31/source/oas31-schema.json" ;;
    arazzo)   echo "$PROJECT_ROOT/fixtures/arazzo/source/arazzo-schema.json" ;;
    asyncapi) echo "$PROJECT_ROOT/fixtures/asyncapi/source/asyncapi-2.6-schema-local.json" ;;
    *) echo "UNKNOWN"; return 1 ;;
  esac
}

spec_output() {
  case "$1" in
    oas31)    echo "$PROJECT_ROOT/fixtures/oas31/openai-strict" ;;
    arazzo)   echo "$PROJECT_ROOT/fixtures/arazzo/openai-strict" ;;
    asyncapi) echo "$PROJECT_ROOT/fixtures/asyncapi/openai-strict" ;;
    *) echo "UNKNOWN"; return 1 ;;
  esac
}

spec_url() {
  case "$1" in
    oas31)    echo "https://spec.openapis.org/oas/3.1/schema/2022-10-07" ;;
    arazzo)   echo "https://raw.githubusercontent.com/OAI/Arazzo-Specification/main/_archive_/schemas/v1.0/schema.json" ;;
    asyncapi) echo "https://raw.githubusercontent.com/asyncapi/spec-json-schemas/master/schemas/2.6.0-without-\$id.json" ;;
    *) echo "UNKNOWN"; return 1 ;;
  esac
}

ALL_SPECS="oas31 arazzo asyncapi"

# ── Parse arguments ─────────────────────────────────────────────────────────

REQUESTED_SPEC="${1:-all}"

if [[ "$REQUESTED_SPEC" == "all" ]]; then
  SPECS="$ALL_SPECS"
else
  if ! spec_source "$REQUESTED_SPEC" >/dev/null 2>&1; then
    echo "ERROR: Unknown spec '$REQUESTED_SPEC'. Valid: $ALL_SPECS | all"
    exit 1
  fi
  SPECS="$REQUESTED_SPEC"
fi

# ── Step 1: Pre-flight checks ───────────────────────────────────────────────

if ! command -v jq >/dev/null 2>&1; then
  echo >&2 "ERROR: jq is required but not installed."
  exit 1
fi

for spec in $SPECS; do
  source_path="$(spec_source "$spec")"
  if [[ ! -f "$source_path" ]]; then
    echo "ERROR: Source schema not found for $spec at $source_path"
    echo "Download it from: $(spec_url "$spec")"
    exit 1
  fi
done

# ── Step 2: Build the CLI ────────────────────────────────────────────────────

echo "🔧 Building CLI (release)..."
cargo build --release -p json-schema-llm --manifest-path "$PROJECT_ROOT/Cargo.toml" 2>&1 | tail -3

# ── Step 3: Generate fixtures per spec ───────────────────────────────────────

generate_spec() {
  local spec="$1"
  local source_path
  local output
  source_path="$(spec_source "$spec")"
  output="$(spec_output "$spec")"

  echo ""
  echo "════════════════════════════════════════════════"
  echo "  ⚙️  Generating: $spec"
  echo "  📄 Source: $source_path"
  echo "════════════════════════════════════════════════"

  # Clean previous output
  if [[ -d "$output" ]]; then
    echo "🗑️  Removing previous output: $output"
    rm -rf "$output"
  fi


  # Generate
  STDERR_FILE=$(mktemp)
  "$CLI" convert "$source_path" --output-dir "$output" 2>"$STDERR_FILE"

  # Report component errors (expected for some recursive/unsupported schemas)
  if grep -q "Component error" "$STDERR_FILE"; then
    echo ""
    echo "⚠️  Component errors (expected for recursive/unsupported schemas):"
    grep "Component error" "$STDERR_FILE" | while read -r line; do
      echo "  $line"
    done
    echo ""
  fi
  rm -f "$STDERR_FILE"

  # Validate JSON files
  echo "🔍 Validating generated JSON files..."
  local total_files=0
  local invalid_files=0

  while IFS= read -r -d '' file; do
    total_files=$((total_files + 1))
    if ! jq . "$file" > /dev/null 2>&1; then
      echo "  ❌ Invalid JSON: $file"
      invalid_files=$((invalid_files + 1))
    fi
  done < <(find "$output" -name "*.json" -print0)

  if [[ $total_files -eq 0 ]]; then
    echo "ERROR: No JSON files generated for $spec"
    exit 1
  fi

  if [[ $invalid_files -gt 0 ]]; then
    echo "ERROR: $invalid_files invalid JSON files found for $spec"
    exit 1
  fi

  # Summary
  local component_count
  component_count=$(jq '.components | length' "$output/manifest.json")
  echo "  ✅ $spec: $component_count components, $total_files files"
}

# ── Execute ──────────────────────────────────────────────────────────────────

for spec in $SPECS; do
  generate_spec "$spec"
done

echo ""
echo "════════════════════════════════════════════════"
echo "  ✅ All fixture generation complete"
echo "════════════════════════════════════════════════"
