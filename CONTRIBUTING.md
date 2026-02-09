# Contributing to jsonschema-llm

Thanks for your interest in contributing! This project is in early development and we welcome contributions.

## Development Setup

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (stable, latest)

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
2. **Integration tests** — Full pipeline tests with real schemas (planned)
3. **Doc tests** — Examples in `lib.rs` and `schema_utils.rs`

```bash
# Unit + doc tests
cargo test
```

### Project Structure

```
jsonschema-llm/
├── crates/
│   └── jsonschema-llm-core/   # Core Rust library
│       └── src/
│           ├── lib.rs          # Public API
│           ├── passes/         # One module per pass (p0–p7)
│           ├── codec.rs        # Codec builder
│           ├── rehydrator.rs   # Reverse transforms
│           └── schema_utils.rs # Shared path/traversal utilities
├── cli/                       # CLI binary
├── bindings/                  # Language bindings (not yet implemented)
│   ├── typescript/            # WASM (planned)
│   ├── java/                  # JNI (planned)
│   └── python/                # PyO3 (planned)
└── docs/
    └── algorithm.md           # Formal algorithm spec
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
