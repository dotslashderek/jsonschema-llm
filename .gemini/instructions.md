# jsonschema-llm — Project Context

This is a **Rust monorepo** that converts arbitrary JSON Schemas (Draft 2020-12) into LLM-compatible structured output schemas. It is an open-source project owned by `dotslashderek`.

## Architecture

- **Core library**: `crates/jsonschema-llm-core/` — Rust crate with an 8-pass compiler pipeline
- **CLI**: `cli/` — Rust binary using clap
- **Bindings**: `bindings/` — TypeScript (WASM), Python (PyO3), Java (JNI) — not yet implemented

## Algorithm

The converter runs 8 ordered passes (see `docs/algorithm.md` for the formal spec):

0. **Normalization** — Resolve `$ref`, normalize drafts
1. **Composition** — Merge `allOf` into flat objects
2. **Polymorphism** — Rewrite `oneOf` → `anyOf` (NOT flatten — flattening causes hallucination)
3. **Dictionary** — `Map<K,V>` → `Array<{key, value}>`
4. **Opaque Types** — `{type: object}` (no props) → `{type: string, description: "JSON-encoded..."}`
5. **Recursion** — Break cycles at depth limit
6. **Strict Mode** — `additionalProperties: false`, all props `required`, optionals → `anyOf [T, null]`
7. **Constraints** — Drop unsupported constraints, sort enums default-first

Each pass emits **codec entries** (rehydration metadata) so LLM output can be transformed back to the original shape.

## Key Design Decisions

- Use `anyOf` over discriminator flattening (prevents hallucination — validated in POC testing)
- Use `Cow<Schema>` for clone-on-write efficiency in the recursive descent transformer
- WASM-first design for in-browser use
- JSON Pointers for codec paths (schema-node references, not data-path references)
- Target OpenAI Strict as baseline; Gemini/Claude as relaxed supersets

## Implementation Priority (v0.1)

Build in this order: Pass 6 (Strict) → Pass 3 (Dictionary) → Pass 1 (Composition) → Rehydrator → Pass 2 (Polymorphism) → Pass 4 (Opaque) → Pass 0 (Normalize) → Pass 5 (Recursion) → Pass 7 (Constraints)

## Validated Against

- Test schema (2.5KB) — 7/7 round-trip checks passed
- Gravitee API Definition v4 (29KB, 1216 lines, production) — OpenAI Strict accepted, full round-trip passed

## Development Guidelines

- **Language**: Rust (stable, latest edition)
- **Testing**: Unit tests in each pass module, integration tests with real schemas
- **CI**: GitHub Actions (Rust build + test + clippy + fmt)
- **Git Flow**: Agentic GitHub Flow — all work linked to Issues and PRs
- D prefers to be referred to as D (not "user"). You are T.
