#!/usr/bin/env bash
# -------------------------------------------------------------------
# assemble-npm.sh — Builds a correct dual-target npm package from
# wasm-pack outputs (pkg-nodejs + pkg-web).
#
# Usage:   bash scripts/assemble-npm.sh
# Expects: crates/jsonschema-llm-wasm/pkg-nodejs/
#          crates/jsonschema-llm-wasm/pkg-web/
# Produces: dist/  (ready for `npm publish`)
# -------------------------------------------------------------------
set -euo pipefail

WASM_CRATE="crates/json-schema-llm-wasm"
DIST="dist"

echo "==> Assembling npm package into ${DIST}/"

# 1. Copy Node.js build as the primary package root
rm -rf "${DIST}"
cp -r "${WASM_CRATE}/pkg-nodejs" "${DIST}"

# 2. Copy web build into web/ subdirectory
mkdir -p "${DIST}/web"
cp "${WASM_CRATE}/pkg-web/json_schema_llm_wasm_bg.wasm" "${DIST}/web/"
cp "${WASM_CRATE}/pkg-web/json_schema_llm_wasm.js"      "${DIST}/web/"
cp "${WASM_CRATE}/pkg-web/json_schema_llm_wasm.d.ts"     "${DIST}/web/"

# 3. Patch package.json with jq
TEMP_PKG=$(mktemp)
jq '
  .name = "jsonschema-llm" |
  .description = "Convert JSON Schema to LLM-compatible structured output schemas (WASM)" |
  .license = "Apache-2.0" |
  .repository = {
    "type": "git",
    "url": "https://github.com/dotslashderek/jsonschema-llm.git",
    "directory": "crates/jsonschema-llm-wasm"
  } |
  .keywords = ["json-schema", "llm", "openai", "gemini", "claude", "wasm", "structured-output"] |
  .files = [
    "json_schema_llm_wasm_bg.wasm",
    "json_schema_llm_wasm.js",
    "json_schema_llm_wasm.d.ts",
    "web/",
    "README.md"
  ] |
  .exports = {
    ".": {
      "types": "./json_schema_llm_wasm.d.ts",
      "node": "./json_schema_llm_wasm.js",
      "default": "./web/json_schema_llm_wasm.js"
    },
    "./web": {
      "types": "./web/json_schema_llm_wasm.d.ts",
      "default": "./web/json_schema_llm_wasm.js"
    }
  }
' "${DIST}/package.json" > "${TEMP_PKG}"
mv "${TEMP_PKG}" "${DIST}/package.json"

# 4. Copy npm README
if [ -f "README-npm.md" ]; then
  cp README-npm.md "${DIST}/README.md"
fi

# 5. Clean up wasm-pack artifacts
rm -f "${DIST}/.gitignore"
# Replace wasm-pack placeholder LICENSE with repo's canonical license
cp LICENSE "${DIST}/LICENSE"

echo "==> Done. Package contents:"
ls -la "${DIST}/"
echo ""
echo "==> package.json:"
cat "${DIST}/package.json"
