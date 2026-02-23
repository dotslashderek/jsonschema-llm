# json-schema-llm — Local CI Verification Targets
#
# Usage:
#   make verify-bindings    # Full WASM/WASI binding pipeline
#   make verify-all         # Full CI mirror (check + test + bindings)
#   make test-wasm-smoke    # WASM smoke tests only
#   make build-wasi         # Build WASI binary only
#
# Prerequisites:
#   - Rust stable with wasm32-wasip1 target
#   - wasm-pack (https://rustwasm.github.io/wasm-pack/installer/)
#   - Python 3 with pip
#   - Docker (for wrapper tests)

.PHONY: verify-bindings verify-all build-wasi distribute-wasm test-wasm-smoke test-wasi-host \
        test-wrappers test-engines test-rust check help

# ---------------------------------------------------------------------------
# Meta-targets
# ---------------------------------------------------------------------------

## Run the full WASM/WASI binding verification pipeline
verify-bindings: distribute-wasm test-wasm-smoke test-wasi-host test-wrappers test-engines
	@echo ""
	@echo "✅ All binding verification targets passed!"

## Full CI mirror: check + test + bindings
verify-all: check test-rust verify-bindings
	@echo ""
	@echo "✅ Full CI verification passed!"

# ---------------------------------------------------------------------------
# Build targets
# ---------------------------------------------------------------------------

## Build the WASI universal binary (wasm32-wasip1)
build-wasi:
	@echo "🔨 Building WASI binary..."
	@rustup target list --installed | grep -q wasm32-wasip1 || \
		(echo "❌ wasm32-wasip1 target not installed. Run: rustup target add wasm32-wasip1" && exit 1)
	cargo build --target wasm32-wasip1 --release -p json-schema-llm-wasi
	@echo "✅ WASI binary built: target/wasm32-wasip1/release/json_schema_llm_wasi.wasm"

## Distribute WASM binary to all directories that need physical copies
distribute-wasm: build-wasi
	@echo "📦 Distributing WASM binary to all targets..."
	@mkdir -p bindings/go/wasm
	cp -f target/wasm32-wasip1/release/json_schema_llm_wasi.wasm \
		bindings/go/wasm/json_schema_llm_wasi.wasm
	@echo "  → bindings/go/wasm/ (Go embed)"
	@mkdir -p engine/python/json_schema_llm_engine/wasm
	cp -f target/wasm32-wasip1/release/json_schema_llm_wasi.wasm \
		engine/python/json_schema_llm_engine/wasm/json_schema_llm_wasi.wasm
	@echo "  → engine/python/json_schema_llm_engine/wasm/ (Engine Python bundle)"
	@echo "✅ WASM binary distributed to all targets"

# ---------------------------------------------------------------------------
# Test targets
# ---------------------------------------------------------------------------

## Run WASM smoke tests via wasm-pack (wasm32-unknown-unknown + Node.js)
test-wasm-smoke:
	@echo "🧪 Running WASM smoke tests..."
	@command -v wasm-pack > /dev/null || \
		(echo "❌ wasm-pack not found. Install: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh" && exit 1)
	wasm-pack test --node crates/json-schema-llm-wasm
	@echo "✅ WASM smoke tests passed"

## Run WASI host verification (Python + wasmtime)
test-wasi-host: build-wasi
	@echo "🧪 Running WASI host verification..."
	@command -v python3 > /dev/null || \
		(echo "❌ python3 not found. Install Python 3.12+." && exit 1)
	@python3 -c "import wasmtime" 2>/dev/null || \
		(echo "⚠️  wasmtime not found. Install with: python3 -m pip install wasmtime" && \
		 echo "   (If on a strict PEP 668 system, activate a venv first.)" && \
		 python3 -m pip install wasmtime)
	python3 tests/wasi/host_verify.py
	@echo "✅ WASI host verification passed"

## Run Docker-based polyglot wrapper tests (all 6 languages)
test-wrappers: distribute-wasm
	@echo "🧪 Running Docker wrapper tests..."
	@(docker compose version > /dev/null 2>&1 || docker-compose version > /dev/null 2>&1) || \
		(echo "❌ docker compose not found. Install Docker Desktop." && exit 1)
	./scripts/test-wrappers.sh
	@echo "✅ Docker wrapper tests passed"

## Run engine E2E tests (Python + Java against real WASM)
test-engines: distribute-wasm
	@echo "🧪 Running engine E2E tests..."
	cd engine/python && python -m pytest -v -m e2e
	cd engine/java && mvn test -Dgroups=e2e -q
	@echo "✅ Engine E2E tests passed"

## Run Rust workspace tests (mirrors CI exclusions)
test-rust:
	@echo "🧪 Running Rust workspace tests..."
	cargo test --workspace --exclude json-schema-llm-python --exclude json-schema-llm-wasi --all-targets
	cargo test --workspace --exclude json-schema-llm-python --exclude json-schema-llm-wasi --doc
	@echo "✅ All Rust tests passed"

# ---------------------------------------------------------------------------
# Lint / format targets
# ---------------------------------------------------------------------------

## Run formatting check + clippy (mirrors CI strictness)
check:
	@echo "🔍 Running formatting check..."
	cargo fmt --all -- --check
	@echo "🔍 Running clippy..."
	cargo clippy --workspace --exclude json-schema-llm-python --exclude json-schema-llm-wasi --all-targets -- -D warnings
	@echo "✅ Formatting and linting passed"

# ---------------------------------------------------------------------------
# Help
# ---------------------------------------------------------------------------

## Show available targets
help:
	@echo "json-schema-llm — Local CI Verification"
	@echo ""
	@echo "Meta-targets:"
	@echo "  make verify-bindings   Full WASM/WASI binding pipeline"
	@echo "  make verify-all        Full CI mirror (check + test + bindings)"
	@echo ""
	@echo "Individual targets:"
	@echo "  make build-wasi        Build WASI binary (wasm32-wasip1)"
	@echo "  make distribute-wasm   Build + copy WASM to Go embed + Engine Python"
	@echo "  make test-wasm-smoke   WASM smoke tests (wasm-pack + Node.js)"
	@echo "  make test-wasi-host    WASI host verification (Python + wasmtime)"
	@echo "  make test-wrappers     Docker wrapper tests (6 languages)"
	@echo "  make test-engines      Engine E2E tests (Python + Java vs real WASM)"
	@echo "  make test-rust         Rust workspace tests (unit + doc)"
	@echo "  make check             Formatting + clippy"
	@echo ""
	@echo "Prerequisites:"
	@echo "  - Rust stable + wasm32-wasip1 target"
	@echo "  - wasm-pack"
	@echo "  - Python 3 + pip"
	@echo "  - Docker Desktop"
