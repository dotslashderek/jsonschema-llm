# Future Blockers & Risk Register

Findings from research on similar projects (JSON Schema → LLM, Rust FFI, wasm-pack).
Updated: 2026-02-16.

## Active Risks

| Risk                                                                                                  | Severity  | Impact                                                                                                      | Mitigation                                                                                                                                                       |
| ----------------------------------------------------------------------------------------------------- | --------- | ----------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **wasm-pack sunset** — rustwasm WG archived July 2025                                                 | 🟡 Medium | No new features/fixes upstream; potential bitrot in JS glue code generation                                 | Pin wasm-pack version in CI (`v0.13.1`); monitor alternatives (`wasm-bindgen` CLI directly, `nickel-rs`); our `assemble-npm.sh` already abstracts the build step |
| **Dual-target `.wasm` divergence** — nodejs vs web builds produce different binaries                  | 🟢 Low    | Already handled by shipping both via `assemble-npm.sh`                                                      | CI validation step (#90) catches regressions; `exports` map in `package.json` routes consumers correctly                                                         |
| **`wasm-opt` musl performance** — 10x slower on musl-based Linux (Alpine)                             | 🟢 Low    | Only affects CI build time                                                                                  | Using glibc runners (`ubuntu-latest`); no action needed unless we adopt Alpine images                                                                            |
| **Malformed LLM JSON** — LLMs produce truncated/trailing-comma JSON that consumers pass to our bridge | 🟡 Medium | `convert_json` rejects with `JsonParseError` — correct but unhelpful for consumers who want lenient parsing | Consider recommending `llm_json` crate upstream of our bridge; future option: add `lenient_parse` feature gate                                                   |

## Resolved / Accepted

| Item                                  | Decision                                                                                          | Date       |
| ------------------------------------- | ------------------------------------------------------------------------------------------------- | ---------- |
| WASI universal binary strategy        | Adopted — single `wasm32-wasip1` binary, 6 language wrappers shipped, native bindings retired     | 2026-02-16 |
| ABI incompatibility (wasm32-unknown)  | Resolved — moved to `wasm32-wasip1` target; C ABI via WASI eliminates the `unknown-unknown` issue | 2026-02-16 |
| JSON-string bridge vs typed FFI       | JSON-string bridge (safest v1 for all three binding targets)                                      | 2026-02-10 |
| Error code stability contract         | `#[non_exhaustive]` + `snake_case` serde + stability test in `ffi_tests.rs`                       | 2026-02-10 |
| camelCase vs snake_case in codec JSON | camelCase (matches JS conventions, documented in codec.rs)                                        | 2026-02-10 |
