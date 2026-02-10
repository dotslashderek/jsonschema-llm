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

WASM_CRATE="crates/jsonschema-llm-wasm"
DIST="dist"

echo "==> Assembling npm package into ${DIST}/"

# 1. Copy Node.js build as the primary package root
rm -rf "${DIST}"
cp -r "${WASM_CRATE}/pkg-nodejs" "${DIST}"

# 2. Copy web build into web/ subdirectory
mkdir -p "${DIST}/web"
cp "${WASM_CRATE}/pkg-web/jsonschema_llm_wasm_bg.wasm" "${DIST}/web/"
cp "${WASM_CRATE}/pkg-web/jsonschema_llm_wasm.js"      "${DIST}/web/"
cp "${WASM_CRATE}/pkg-web/jsonschema_llm_wasm.d.ts"     "${DIST}/web/"

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
    "jsonschema_llm_wasm_bg.wasm",
    "jsonschema_llm_wasm.js",
    "jsonschema_llm_wasm.d.ts",
    "web/",
    "README.md"
  ] |
  .exports = {
    ".": {
      "types": "./jsonschema_llm_wasm.d.ts",
      "node": "./jsonschema_llm_wasm.js",
      "default": "./web/jsonschema_llm_wasm.js"
    },
    "./web": {
      "types": "./web/jsonschema_llm_wasm.d.ts",
      "default": "./web/jsonschema_llm_wasm.js"
    }
  }
' "${DIST}/package.json" > "${TEMP_PKG}"
mv "${TEMP_PKG}" "${DIST}/package.json"

# 4. Copy npm README
if [ -f "README-npm.md" ]; then
  cp README-npm.md "${DIST}/README.md"
fi

# 5. Remove wasm-pack artifacts we don't need
rm -f "${DIST}/.gitignore"
rm -f "${DIST}/LICENSE"

echo "==> Done. Package contents:"
ls -la "${DIST}/"
echo ""
echo "==> package.json:"
cat "${DIST}/package.json"
