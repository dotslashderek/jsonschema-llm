# Contributing to jsonschema-llm

Thanks for your interest in contributing! This project is in early development and we welcome contributions.

## Development Setup

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (stable, latest)
- [Node.js](https://nodejs.org/) 20+ and [pnpm](https://pnpm.io/) (for WASM contract tests)
- [Python](https://www.python.org/) 3.9+ and [maturin](https://www.maturin.rs/) (for Python bindings)

### Building

```bash
# Build the core library
cargo build

# Run tests
cargo test

# Run clippy
cargo clippy -- -D warnings

# Check formatting
cargo fmt -- --check
```

### Testing

The project uses a multi-layer testing strategy:

1. **Unit tests** — Rust `#[test]` modules within each pass
2. **E2E tests** — Full pipeline tests with real schemas in `crates/jsonschema-llm-core/tests/`
3. **CLI tests** — End-to-end CLI integration tests in `cli/tests/`
4. **WASM contract tests** — Node.js tests verifying WASM bindings in `tests/contract-node/`
5. **Python acceptance tests** — pytest suite in `crates/jsonschema-llm-python/tests/`
6. **Doc tests** — Examples in `lib.rs` and `schema_utils.rs`
7. **Property tests** — `proptest` strategies in `crates/jsonschema-llm-core/tests/proptest_*.rs`
8. **Fuzzing** — `cargo-fuzz` harness in `fuzz/` (requires nightly, not part of workspace)

```bash
# Core Rust tests (includes proptests)
cargo test

# Python bindings
cd crates/jsonschema-llm-python && maturin develop && pytest tests/

# WASM contract tests
wasm-pack build crates/jsonschema-llm-wasm --target nodejs --out-dir ../../tests/contract-node/pkg
cd tests/contract-node && pnpm test

# Fuzzing (nightly only, not run in CI)
cargo +nightly fuzz run fuzz_convert -- -max_total_time=60
```

### Project Structure

```
jsonschema-llm/
├── crates/
│   ├── jsonschema-llm-core/     # Core Rust library
│   │   └── src/
│   │       ├── lib.rs            # Public API
│   │       ├── passes/           # One module per pass (p0–p7)
│   │       ├── codec.rs          # Codec builder
│   │       ├── rehydrator.rs     # Reverse transforms
│   │       └── schema_utils.rs   # Shared path/traversal utilities
│   ├── jsonschema-llm-wasm/     # TypeScript/JS WASM bindings
│   └── jsonschema-llm-python/   # Python PyO3 bindings
├── cli/                         # CLI binary
├── fuzz/                        # cargo-fuzz harness (standalone, nightly)
├── tests/
│   └── contract-node/           # WASM contract tests (Node.js)
└── docs/
    └── algorithm.md             # Formal algorithm spec
```

## Pull Request Process

1. Fork the repository and create a feature branch
2. Write tests for your changes
3. Ensure `cargo test` and `cargo clippy` pass
4. Submit a PR with a clear description of the change

## Code Style

- Follow standard Rust conventions (`rustfmt`, `clippy`)
- Use `serde_json::Value` for schema manipulation
- Document all public APIs with doc comments
- Each pass should be a self-contained module in `crates/jsonschema-llm-core/src/passes/`

## Reporting Issues

Please include:

- The input JSON Schema (or a minimal reproduction)
- Expected vs actual output
- Target provider (OpenAI, Gemini, Claude)
- `jsonschema-llm` version
