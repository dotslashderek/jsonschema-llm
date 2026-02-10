# Stress Test & E2E Verification Report

v0.1.0 • Branch: `stress-test/chaos-monkey` • Feb 2026

## Executive Summary

Executed a comprehensive stress test suite against `jsonschema-llm` targeting `openai-strict`.

- **Generated:** 100 unique, complex JSON Schema fixtures.
- **Tested:** CLI (conversion + rehydration) and JS/WASM Bindings (chatbot flow).
- **Result:** ~60% Pass Rate. Identified 4 critical failure modes specific to OpenAI Strict Mode compliance.

## Methodology

1.  **Schema Generation**: `scripts/stress/generate_basic_stress.py`
    - Structural variation: Depth (5-50), Width (2-5)
    - Polymorphism: `oneOf`, `allOf`, `anyOf` combinations.
    - Edge Cases: Boolean schemas, unique Unicode keys, reserved words.
    - Initial corpus: 100 files in `tests/schemas/stress/`.

2.  **CLI Stress Test**: `scripts/stress/run_cli_test.py`
    - Full round-trip: Schema -> Convert -> OpenAI (Mock) -> Rehydrate -> Validate.
    - Verified strict schema compliance and rehydration fidelity.

3.  **Chatbot Integration**: `examples/stress-test-bot`
    - TypeScript application using `jsonschema-llm` WASM bindings.
    - Verified real-world usage patterns with `openai` SDK.

## Key Findings & Failures

### 1. Root Schema Type Violation

**Severity:** Critical
**Error:** `Invalid schema for response_format ... schema must be a JSON Schema of 'type: "object"', got 'type: "array"'.`
**Description:** OpenAI Strict Mode mandates the root schema be an `object`.
**Action:** `jsonschema-llm` must detect non-object roots (arrays, primitives, enums) and wrap them (e.g., `{ "value": <schema> }`) or error early.

### 2. Nesting Depth Limit Exceeded

**Severity:** High
**Error:** `Invalid schema for response_format ... 11 levels of nesting exceeds limit of 10.`
**Description:** OpenAI enforces a strict nesting depth limit (appear to be ~10 or 5 depending on model/context). Deeply nested schemas (`combo_depth_10_width_2.json`) fail at the API level even if valid JSON Schema.
**Action:** `jsonschema-llm` should provide a warning or an option to flatten/truncate schemas that exceed target-specific depth limits.

### 3. Heterogeneous Enums

**Severity:** Medium
**Error:** `Invalid schema ...: enum values must be of the same type.`
**Description:** Schemas with `enum: ["red", 1, true]` fail validation.
**Action:** The converter must normalize these, likely by stringifying all values or splitting the enum into a `oneOf` of consts (though `oneOf` consts might be verbose, stringification is safer for LLM output).

### 4. Boolean/Empty Schemas

**Severity:** Medium
**Error:** `additionalProperties is required to be supplied and to be false.`
**Description:** `true`, `false`, and `{}` schemas fail because they lack the strict object constraints OpenAI demands.
**Action:** `jsonschema-llm` must compile these to their strict equivalents:

- `{}` -> `{"type": "object", "additionalProperties": false, "properties": {}}` (matches nothing extra? wait, `{}` accepts everything. Strict equivalent for "any JSON" is hard. Maybe `{"type": ["string", "number", "boolean", "object", "array", "null"]}`?)
- `true` -> logic needed.
- `false` -> `{"not": {}}` (not supported by OpenAI?).

## Verification Status

- **Recursion:** ✅ PASS. The referenced-based recursion handling works perfectly for supported depths.
- **Polymorphism:** ⚠️ MIXED. `oneOf` works, but specific `allOf` merges might be hitting edge cases.
- **Rehydration:** ✅ PASS. For schemas that successfully converted and returned data, rehydration was 100% accurate.

## Artifacts

- **Schemas:** `tests/schemas/stress/*.json`
- **CLI Runner:** `scripts/stress/run_cli_test.py`
- **Chatbot:** `examples/stress-test-bot/`
- **WASM Build:** `dist/` (npm package)
