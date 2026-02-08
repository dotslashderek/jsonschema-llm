# jsonschema-llm — Project Context

This is a **Rust monorepo** that converts arbitrary JSON Schemas (Draft 2020-12) into LLM-compatible structured output schemas. It is an open-source project owned by `dotslashderek`.

## Architecture

- **Core library**: `crates/jsonschema-llm-core/` — Rust crate with an 8-pass compiler pipeline
- **CLI**: `cli/` — Rust binary using clap (stub)
- **Bindings**: `bindings/` — TypeScript (WASM), Python (PyO3), Java (JNI) — not yet implemented

## Algorithm

The converter runs 8 ordered passes (see `docs/algorithm.md` for the formal spec):

0. **Normalization** — Resolve `$ref` (root-relative only), normalize drafts, `items` array → `prefixItems`, `definitions` → `$defs`
1. **Composition** — Merge `allOf` into flat objects
2. **Polymorphism** — Rewrite `oneOf` → `anyOf` (NOT flatten — flattening causes hallucination)
3. **Dictionary** — `Map<K,V>` → `Array<{key, value}>`
4. **Opaque Types** — `{type: object}` (no props) → `{type: string, description: "JSON-encoded..."}`
5. **Recursion** — Break cycles at depth limit
6. **Strict Mode** — `additionalProperties: false`, all props `required`, optionals → `anyOf [T, null]`
7. **Constraints** — Drop unsupported constraints, sort enums default-first

Each pass emits **codec entries** (rehydration metadata) so LLM output can be transformed back to the original shape.

## Implementation Status

Passes 0–4 and 6 are **fully implemented** with tests. Passes 5 and 7 remain as stubs. The `convert()` pipeline function and CLI are not yet wired. The rehydrator is complete with advisory warnings.

## Key Design Decisions

- Use `anyOf` over discriminator flattening (prevents hallucination — validated in POC testing)
- Use `serde_json::Value` with explicit cloning (simpler than `Cow<Schema>` given pass-by-pass design)
- WASM-first design for in-browser use
- JSON Pointers for codec paths (schema-node references, not data-path references)
- Target OpenAI Strict as baseline; Gemini/Claude as relaxed supersets

## Validated Against

- Test schema (2.5KB) — 7/7 round-trip checks passed
- Gravitee API Definition v4 (29KB, 1216 lines, production) — OpenAI Strict accepted, full round-trip passed

## Development Guidelines

- **Language**: Rust (stable, latest edition)
- **Testing**: Unit tests in each pass module, integration tests with real schemas
- **CI**: GitHub Actions (Rust build + test + clippy + fmt)
- **Git Flow**: Agentic GitHub Flow — all work linked to Issues and PRs
- D prefers to be referred to as D (not "user"). You are T.
