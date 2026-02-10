# Stress Test Report — jsonschema-llm

> **Status: STRUCTURALLY SOUND — COMPLIANCE GAPS NEED WORK**

## Run Metadata

| Field            | Value                                |
| ---------------- | ------------------------------------ |
| **Model**        | `gpt-4o-mini`                        |
| **API Mode**     | OpenAI (Live API)                    |
| **Seed**         | `42`                                 |
| **Date**         | 2025-02-09                           |
| **Schema Count** | 100 (stress + real-world)            |
| **CLI Binary**   | `target/release/jsonschema-llm`      |
| **Commit**       | `c86374f` (stress-test/chaos-monkey) |

## Methodology

The stress test pipeline exercises the full roundtrip:

1. **Convert** — `jsonschema-llm convert` transforms input schema to OpenAI-strict format with codec
2. **Generate** — Live OpenAI API call (`gpt-4o-mini`) produces data matching the converted schema
3. **Rehydrate** — `jsonschema-llm rehydrate` restores original structure using codec
4. **Validate** — `jsonschema.validate()` (Python) or AJV (TypeScript) checks rehydrated data against original schema

> [!NOTE]
> All API calls are **live** — this is NOT a mock test. Results depend on model behavior, rate limits, and API availability.

## Results Summary

| Stage     | Pass | Fail | Notes                                                           |
| --------- | ---- | ---- | --------------------------------------------------------------- |
| Convert   | ~60  | ~40  | Root-schema, depth, heterogeneous enum, boolean schema failures |
| OpenAI    | ~55  | ~5   | Refusals on malformed/unsatisfiable schemas                     |
| Rehydrate | ~53  | ~2   | Codec edge cases                                                |
| Validate  | ~50  | ~3   | Rehydration drift on nested polymorphism                        |

**Overall pass rate: ~50/100 (~50%)**

> [!WARNING]
> The previous version of this report described the system as "HEALTHY" with a ~60% pass rate. That framing was overly optimistic given 4 critical/high failure modes. The failures represent real gaps in strict-mode compliance that need targeted work.

## Failure Analysis (by severity)

### P0 — Critical

| Issue                               | Count | Root Cause                                                | Tracking |
| ----------------------------------- | ----- | --------------------------------------------------------- | -------- |
| Root `$ref` schemas fail conversion | ~15   | Engine doesn't support root-level `$ref` without wrapping | #94      |
| Depth limit exceeded                | ~10   | Deep nesting hits recursion guard                         | #95      |
| Heterogeneous enum values           | ~8    | Mixed-type enums not lowered correctly                    | #96      |
| Boolean schemas (`true`/`false`)    | ~4    | Not handled by normalize pass                             | #97      |

### P1 — Accepted Failures

| Issue                               | Count | Notes                                                |
| ----------------------------------- | ----- | ---------------------------------------------------- |
| `edge_false` (unsatisfiable schema) | 1     | Expected fail — `false` schema rejects all instances |
| `edge_empty` (empty schema)         | 1     | Accepted — semantics unclear for LLM providers       |

### P2 — Provider-Specific

| Issue                             | Count | Notes                                             |
| --------------------------------- | ----- | ------------------------------------------------- |
| OpenAI refusal on complex schemas | ~3    | Some deeply nested schemas exceed provider limits |
| Rehydration warnings (non-fatal)  | ~5    | Codec handles gracefully, data still valid        |

## Remediation Plan

| Priority | Action                                                 | Issue |
| -------- | ------------------------------------------------------ | ----- |
| 1        | Implement root `$ref` wrapping in normalize pass       | #94   |
| 2        | Add configurable depth limit with graceful degradation | #95   |
| 3        | Lower heterogeneous enums to `oneOf` + `const`         | #96   |
| 4        | Handle boolean schemas in normalize pass               | #97   |
| 5        | Add dedicated `provider_compat` pass                   | #102  |
| 6        | Implement stable error taxonomy                        | #103  |

## Reproducing

```bash
# Build CLI
cargo build --release

# Generate stress schemas (deterministic)
python scripts/stress/generate_basic_stress.py --seed 42

# Run CLI stress test
python scripts/stress/run_cli_test.py \
  --bin target/release/jsonschema-llm \
  --schemas tests/schemas/stress \
  --model gpt-4o-mini \
  --timeout-subprocess 30 \
  --timeout-api 60

# Run chatbot stress test (TS bindings)
# First, build the JS bindings:
wasm-pack build crates/jsonschema-llm-wasm --target nodejs --out-dir ../../dist
cd examples/stress-test-bot
pnpm install && pnpm start -- --count 20 --seed 42 --model gpt-4o-mini
```

## Original Report

The original unmodified report is preserved at [`original_stress_test_report.md`](./original_stress_test_report.md) for historical reference.
