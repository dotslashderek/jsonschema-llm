# Contributing to jsonschema-llm

Thanks for your interest in contributing! This project is in early development and we welcome contributions.

## Development Setup

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (stable, latest)
- [Python 3.11+](https://www.python.org/) (for PyO3 bindings and testing)
- [Node.js 20+](https://nodejs.org/) (for WASM bindings)
- [pnpm](https://pnpm.io/) (for TypeScript workspace)

### Building

```bash
# Build the core library
cargo build

# Run tests
cargo test

# Build CLI
cargo build --bin jsonschema-llm

# Build WASM bindings
cd bindings/typescript && wasm-pack build
```

### Testing

The project uses a multi-layer testing strategy:

1. **Unit tests** — Rust `#[test]` modules within each pass
2. **Integration tests** — Full pipeline tests with real schemas
3. **Round-trip tests** — Convert → mock LLM output → rehydrate → verify original shape
4. **Provider validation** — Live tests against OpenAI/Gemini APIs (requires API keys)

```bash
# Unit + integration tests
cargo test

# Round-trip tests (Python)
cd bindings/python && python -m pytest
```

### Project Structure

```
jsonschema-llm/
├── crates/
│   └── jsonschema-llm-core/   # Core Rust library
│       └── src/
│           ├── lib.rs          # Public API
│           ├── passes/         # One module per pass
│           ├── codec.rs        # Codec builder
│           └── rehydrator.rs   # Reverse transforms
├── cli/                       # CLI binary
├── bindings/
│   ├── typescript/            # WASM bindings
│   ├── java/                  # JNI bindings
│   └── python/                # PyO3 bindings
├── docs/
│   └── algorithm.md           # Formal algorithm spec
└── tests/
    └── schemas/               # Test schemas
```

## Pull Request Process

1. Fork the repository and create a feature branch
2. Write tests for your changes
3. Ensure `cargo test` and `cargo clippy` pass
4. Submit a PR with a clear description of the change

## Code Style

- Follow standard Rust conventions (`rustfmt`, `clippy`)
- Use `Cow<'a, T>` for clone-on-write where applicable
- Document all public APIs with doc comments
- Each pass should be a self-contained module in `crates/jsonschema-llm-core/src/passes/`

## Reporting Issues

Please include:

- The input JSON Schema (or a minimal reproduction)
- Expected vs actual output
- Target provider (OpenAI, Gemini, Claude)
- `jsonschema-llm` version
