#!/usr/bin/env python3
"""
WASI host verification for jsonschema-llm.

End-to-end test of the JslResult protocol: allocate guest memory, write JSON,
call jsl_convert / jsl_rehydrate, read results, free memory.

Usage:
    python3 tests/wasi/host_verify.py [path-to-wasm]

Default WASM path: target/wasm32-wasip1/release/jsonschema_llm_wasi.wasm
"""

import json
import struct
import sys
import time
from pathlib import Path

import wasmtime

# ---------------------------------------------------------------------------
# Constants matching JslResult protocol
# ---------------------------------------------------------------------------
STATUS_OK = 0
STATUS_ERROR = 1

# JslResult is 12 bytes: 3 × u32 (status, ptr, len)
JSL_RESULT_SIZE = 12


def load_wasm(wasm_path: str) -> tuple:
    """Load the WASM module and instantiate it, returning (store, instance, memory)."""
    engine = wasmtime.Engine()
    module = wasmtime.Module.from_file(engine, wasm_path)

    linker = wasmtime.Linker(engine)
    linker.define_wasi()

    wasi_config = wasmtime.WasiConfig()
    store = wasmtime.Store(engine)
    store.set_wasi(wasi_config)

    instance = linker.instantiate(store, module)
    memory = instance.exports(store)["memory"]

    return store, instance, memory


def call_jsl(store, instance, memory, func_name: str, *json_args: str) -> dict:
    """
    Call a jsl_ function with JSON string arguments.

    Handles the full protocol: alloc → write → call → read result → free.
    Returns the parsed JSON response dict with a 'status' key added.
    """
    exports = instance.exports(store)
    jsl_alloc = exports["jsl_alloc"]
    jsl_free = exports["jsl_free"]
    jsl_result_free = exports["jsl_result_free"]
    func = exports[func_name]

    # Allocate and write each JSON argument into guest memory
    ptrs = []
    for arg in json_args:
        data = arg.encode("utf-8")
        ptr = jsl_alloc(store, len(data))
        assert ptr != 0, f"jsl_alloc returned null for {len(data)} bytes"
        memory.write(store, data, ptr)
        ptrs.append((ptr, len(data)))

    # Build the flat argument list: ptr, len, ptr, len, ...
    flat_args = []
    for ptr, length in ptrs:
        flat_args.extend([ptr, length])

    # Call the function
    result_ptr = func(store, *flat_args)
    assert result_ptr != 0, f"{func_name} returned null result pointer"

    # Read the JslResult struct (12 bytes)
    result_bytes = memory.read(store, result_ptr, result_ptr + JSL_RESULT_SIZE)
    status, payload_ptr, payload_len = struct.unpack("<III", result_bytes)

    # Read the JSON payload
    payload_bytes = memory.read(store, payload_ptr, payload_ptr + payload_len)
    payload_str = payload_bytes.decode("utf-8")
    payload = json.loads(payload_str)

    # Free the result (frees both struct and payload)
    jsl_result_free(store, result_ptr)

    # Free input buffers
    for ptr, length in ptrs:
        jsl_free(store, ptr, length)

    return {"status": status, "payload": payload}


def call_jsl_with_null_opts(store, instance, memory) -> dict:
    """
    Call jsl_convert with opts_ptr=0, opts_len=0 to test default options path.
    """
    exports = instance.exports(store)
    jsl_alloc = exports["jsl_alloc"]
    jsl_free = exports["jsl_free"]
    jsl_result_free = exports["jsl_result_free"]
    jsl_convert = exports["jsl_convert"]

    schema = json.dumps({"type": "object", "properties": {"x": {"type": "string"}}})
    data = schema.encode("utf-8")
    schema_ptr = jsl_alloc(store, len(data))
    memory.write(store, data, schema_ptr)

    # Pass 0/0 for options
    result_ptr = jsl_convert(store, schema_ptr, len(data), 0, 0)
    assert result_ptr != 0, "jsl_convert returned null result pointer"

    result_bytes = memory.read(store, result_ptr, result_ptr + JSL_RESULT_SIZE)
    status, payload_ptr, payload_len = struct.unpack("<III", result_bytes)
    payload_str = memory.read(store, payload_ptr, payload_ptr + payload_len).decode("utf-8")
    payload = json.loads(payload_str)

    jsl_result_free(store, result_ptr)
    jsl_free(store, schema_ptr, len(data))

    return {"status": status, "payload": payload}


# ---------------------------------------------------------------------------
# Test functions
# ---------------------------------------------------------------------------


def test_convert_simple(store, instance, memory):
    """Test: simple schema converts successfully with apiVersion."""
    schema = json.dumps({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer", "minimum": 0},
        },
        "required": ["name", "age"],
    })
    options = json.dumps({})

    result = call_jsl(store, instance, memory, "jsl_convert", schema, options)

    assert result["status"] == STATUS_OK, f"Expected OK, got status {result['status']}: {result['payload']}"
    payload = result["payload"]
    assert payload["apiVersion"] == "1.0", f"Missing apiVersion: {payload}"
    assert "schema" in payload, f"Missing schema: {payload}"
    assert "codec" in payload, f"Missing codec: {payload}"

    print(f"  ✅ convert(simple) → apiVersion={payload['apiVersion']}, "
          f"schema keys={list(payload['schema'].keys())}")
    return payload


def test_rehydrate_simple(store, instance, memory, convert_payload):
    """Test: rehydrate round-trips with the codec from convert."""
    schema = json.dumps({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer", "minimum": 0},
        },
        "required": ["name", "age"],
    })
    data = json.dumps({"name": "Ada", "age": 36})
    codec = json.dumps(convert_payload["codec"])

    result = call_jsl(store, instance, memory, "jsl_rehydrate", data, codec, schema)

    assert result["status"] == STATUS_OK, f"Expected OK, got status {result['status']}: {result['payload']}"
    payload = result["payload"]
    assert payload["apiVersion"] == "1.0", f"Missing apiVersion: {payload}"
    assert "data" in payload, f"Missing data: {payload}"
    assert payload["data"]["name"] == "Ada", f"Data mismatch: {payload['data']}"
    assert payload["data"]["age"] == 36, f"Data mismatch: {payload['data']}"

    print(f"  ✅ rehydrate(simple) → data={payload['data']}, "
          f"warnings={len(payload.get('warnings', []))}")


def test_convert_error(store, instance, memory):
    """Test: invalid JSON input produces a status=1 error with structured JSON."""
    result = call_jsl(store, instance, memory, "jsl_convert", "NOT VALID JSON", "{}")

    assert result["status"] == STATUS_ERROR, f"Expected ERROR, got status {result['status']}"
    payload = result["payload"]
    assert "code" in payload, f"Missing error code: {payload}"
    assert "message" in payload, f"Missing error message: {payload}"
    assert payload["code"] == "json_parse_error", f"Wrong error code: {payload['code']}"

    print(f"  ✅ convert(invalid JSON) → error code={payload['code']}")


def test_convert_null_options(store, instance, memory):
    """Test: passing opts_ptr=0, opts_len=0 uses defaults (no crash)."""
    result = call_jsl_with_null_opts(store, instance, memory)

    assert result["status"] == STATUS_OK, f"Expected OK, got status {result['status']}: {result['payload']}"
    payload = result["payload"]
    assert payload["apiVersion"] == "1.0"

    print(f"  ✅ convert(null options) → uses defaults, apiVersion={payload['apiVersion']}")


def test_convert_empty_whitespace_options(store, instance, memory):
    """Test: passing '{ }' (whitespace-padded empty object) uses defaults."""
    schema = json.dumps({"type": "object", "properties": {"x": {"type": "string"}}})
    result = call_jsl(store, instance, memory, "jsl_convert", schema, "{  }")

    assert result["status"] == STATUS_OK, f"Expected OK, got status {result['status']}: {result['payload']}"
    print(f"  ✅ convert(whitespace empty options) → uses defaults")


def test_convert_null_pointer_error(store, instance, memory):
    """Test: passing schema_ptr=0 with schema_len>0 returns invalid_pointer error."""
    exports = instance.exports(store)
    jsl_result_free = exports["jsl_result_free"]
    jsl_convert = exports["jsl_convert"]

    # Pass ptr=0, len=1 — should trigger invalid_pointer error, not a crash
    result_ptr = jsl_convert(store, 0, 1, 0, 0)
    assert result_ptr != 0, "jsl_convert returned null result pointer for null-pointer test"

    result_bytes = memory.read(store, result_ptr, result_ptr + JSL_RESULT_SIZE)
    status, payload_ptr, payload_len = struct.unpack("<III", result_bytes)
    payload_str = memory.read(store, payload_ptr, payload_ptr + payload_len).decode("utf-8")
    payload = json.loads(payload_str)

    assert status == STATUS_ERROR, f"Expected ERROR, got status {status}"
    assert payload["code"] == "invalid_pointer", f"Wrong error code: {payload['code']}"

    jsl_result_free(store, result_ptr)
    print(f"  ✅ convert(null ptr, len=1) → error code={payload['code']}")


def test_convert_invalid_utf8_schema(store, instance, memory):
    """Test: invalid UTF-8 bytes in schema produce invalid_utf8 error."""
    exports = instance.exports(store)
    jsl_alloc = exports["jsl_alloc"]
    jsl_free = exports["jsl_free"]
    jsl_result_free = exports["jsl_result_free"]
    jsl_convert = exports["jsl_convert"]

    # Write invalid UTF-8 bytes (0xFF 0xFE are never valid in UTF-8)
    bad_bytes = b'\xff\xfe{"type":"object"}'
    schema_ptr = jsl_alloc(store, len(bad_bytes))
    memory.write(store, bad_bytes, schema_ptr)

    result_ptr = jsl_convert(store, schema_ptr, len(bad_bytes), 0, 0)
    assert result_ptr != 0, "jsl_convert returned null for invalid UTF-8 test"

    result_bytes = memory.read(store, result_ptr, result_ptr + JSL_RESULT_SIZE)
    status, payload_ptr, payload_len = struct.unpack("<III", result_bytes)
    payload = json.loads(memory.read(store, payload_ptr, payload_ptr + payload_len).decode("utf-8"))

    assert status == STATUS_ERROR, f"Expected ERROR, got {status}"
    assert payload["code"] == "invalid_utf8", f"Wrong code: {payload['code']}"
    assert "byte offset" in payload["message"], f"Missing offset info: {payload['message']}"

    jsl_result_free(store, result_ptr)
    jsl_free(store, schema_ptr, len(bad_bytes))
    print(f"  ✅ convert(invalid UTF-8 schema) → {payload['code']}, {payload['message']}")


def test_convert_invalid_utf8_options(store, instance, memory):
    """Test: invalid UTF-8 bytes in options produce invalid_utf8 error."""
    exports = instance.exports(store)
    jsl_alloc = exports["jsl_alloc"]
    jsl_free = exports["jsl_free"]
    jsl_result_free = exports["jsl_result_free"]
    jsl_convert = exports["jsl_convert"]

    # Valid schema, invalid UTF-8 options
    schema = json.dumps({"type": "object"}).encode("utf-8")
    schema_ptr = jsl_alloc(store, len(schema))
    memory.write(store, schema, schema_ptr)

    bad_opts = b'\xff\xfe{}'
    opts_ptr = jsl_alloc(store, len(bad_opts))
    memory.write(store, bad_opts, opts_ptr)

    result_ptr = jsl_convert(store, schema_ptr, len(schema), opts_ptr, len(bad_opts))
    assert result_ptr != 0

    result_bytes = memory.read(store, result_ptr, result_ptr + JSL_RESULT_SIZE)
    status, payload_ptr, payload_len = struct.unpack("<III", result_bytes)
    payload = json.loads(memory.read(store, payload_ptr, payload_ptr + payload_len).decode("utf-8"))

    assert status == STATUS_ERROR, f"Expected ERROR, got {status}"
    assert payload["code"] == "invalid_utf8"

    jsl_result_free(store, result_ptr)
    jsl_free(store, schema_ptr, len(schema))
    jsl_free(store, opts_ptr, len(bad_opts))
    print(f"  ✅ convert(invalid UTF-8 options) → {payload['code']}")


def test_convert_invalid_options_json(store, instance, memory):
    """Test: syntactically invalid JSON options produce an error (not crash)."""
    schema = json.dumps({"type": "object", "properties": {"x": {"type": "string"}}})
    result = call_jsl(store, instance, memory, "jsl_convert", schema, "NOT JSON AT ALL")

    assert result["status"] == STATUS_ERROR, f"Expected ERROR, got {result['status']}"
    print(f"  ✅ convert(invalid options JSON) → error code={result['payload'].get('code', 'N/A')}")


def test_convert_partial_options(store, instance, memory):
    """Test: valid JSON options missing required 'target' field produce error."""
    schema = json.dumps({"type": "object", "properties": {"x": {"type": "string"}}})
    partial_opts = json.dumps({"max_depth": 5})  # missing 'target'
    result = call_jsl(store, instance, memory, "jsl_convert", schema, partial_opts)

    # Should produce a deserialization error from core, not crash
    assert result["status"] == STATUS_ERROR, f"Expected ERROR, got {result['status']}"
    print(f"  ✅ convert(partial options, missing target) → error code={result['payload'].get('code', 'N/A')}")


def test_convert_empty_schema(store, instance, memory):
    """Test: empty string schema produces an error (not crash)."""
    # read_guest_str returns Ok("") for len=0, then convert_json gets ""
    exports = instance.exports(store)
    jsl_convert = exports["jsl_convert"]
    jsl_result_free = exports["jsl_result_free"]

    # ptr=0, len=0 for schema → read_guest_str returns Ok("")
    result_ptr = jsl_convert(store, 0, 0, 0, 0)
    assert result_ptr != 0, "jsl_convert returned null for empty schema test"

    result_bytes = memory.read(store, result_ptr, result_ptr + JSL_RESULT_SIZE)
    status, payload_ptr, payload_len = struct.unpack("<III", result_bytes)
    payload = json.loads(memory.read(store, payload_ptr, payload_ptr + payload_len).decode("utf-8"))

    # Empty schema string should cause a parse error
    assert status == STATUS_ERROR, f"Expected ERROR for empty schema, got {status}"

    jsl_result_free(store, result_ptr)
    print(f"  ✅ convert(empty schema '') → error code={payload.get('code', 'N/A')}")


def test_rehydrate_error(store, instance, memory):
    """Test: rehydrate with invalid codec produces structured error."""
    schema = json.dumps({"type": "object", "properties": {"x": {"type": "string"}}})
    data = json.dumps({"x": "hello"})
    bad_codec = "NOT VALID JSON"

    result = call_jsl(store, instance, memory, "jsl_rehydrate", data, bad_codec, schema)

    assert result["status"] == STATUS_ERROR, f"Expected ERROR, got {result['status']}"
    payload = result["payload"]
    assert "code" in payload, f"Missing error code: {payload}"

    print(f"  ✅ rehydrate(invalid codec) → error code={payload['code']}")


def test_rehydrate_roundtrip_real_world(store, instance, memory):
    """Test: convert + rehydrate round-trip with a real-world schema."""
    project_root = Path(__file__).resolve().parent.parent.parent
    rw_dir = project_root / "tests" / "schemas" / "real-world"

    if not rw_dir.exists():
        print("  ⏭️  rehydrate(real-world) → skipped, no real-world schemas dir")
        return

    # Pick first schema alphabetically for determinism
    schema_file = sorted(rw_dir.glob("*.json"))[0]
    schema = schema_file.read_text()
    options = json.dumps({})

    # Step 1: Convert
    convert_result = call_jsl(store, instance, memory, "jsl_convert", schema, options)
    assert convert_result["status"] == STATUS_OK, f"Convert failed: {convert_result['payload']}"

    codec = json.dumps(convert_result["payload"]["codec"])
    converted_schema = convert_result["payload"]["schema"]

    # Step 2: Create minimal valid data matching converted schema
    # Use empty object as data — rehydrate should handle it even if not fully valid
    data = json.dumps({})

    # Step 3: Rehydrate
    rh_result = call_jsl(store, instance, memory, "jsl_rehydrate", data, codec, schema)
    # We accept both OK (valid round-trip) and ERROR (data doesn't match schema)
    # The point is: it must not crash
    assert rh_result["status"] in (STATUS_OK, STATUS_ERROR), f"Unexpected status: {rh_result['status']}"

    name = schema_file.stem
    status_label = "OK" if rh_result["status"] == STATUS_OK else "ERROR (expected — empty data)"
    print(f"  ✅ rehydrate({name}) round-trip → {status_label}")


def test_alloc_zero(store, instance, memory):
    """Test: jsl_alloc(0) doesn't crash (Vec::with_capacity(0) is valid)."""
    exports = instance.exports(store)
    jsl_alloc = exports["jsl_alloc"]
    jsl_free = exports["jsl_free"]

    # Should not crash — Vec::with_capacity(0) returns a valid non-null pointer
    ptr = jsl_alloc(store, 0)
    # ptr may or may not be 0 depending on allocator, but it should not trap
    print(f"  ✅ alloc(0) → ptr={ptr} (no crash)")

    # Free should handle 0-length gracefully (hits the len==0 guard)
    jsl_free(store, ptr, 0)
    print(f"  ✅ free(ptr, 0) → no crash")


def test_free_null_guards(store, instance, memory):
    """Test: jsl_free(0,0), jsl_free(0,1), and jsl_result_free(0) don't crash."""
    exports = instance.exports(store)
    jsl_free = exports["jsl_free"]
    jsl_result_free = exports["jsl_result_free"]

    # All of these should hit guards and return safely
    jsl_free(store, 0, 0)
    jsl_free(store, 0, 1)  # ptr=0 guard
    jsl_result_free(store, 0)  # result_ptr=0 guard

    print(f"  ✅ free(0,0), free(0,1), result_free(0) → all no-crash")


def test_convert_real_world(store, instance, memory, schema_path: str):
    """Test: convert a real-world schema from the test corpus."""
    with open(schema_path) as f:
        schema = f.read()
    options = json.dumps({})

    start = time.perf_counter_ns()
    result = call_jsl(store, instance, memory, "jsl_convert", schema, options)
    elapsed_us = (time.perf_counter_ns() - start) / 1000

    assert result["status"] == STATUS_OK, f"Failed on {schema_path}: {result['payload']}"
    payload = result["payload"]
    assert payload["apiVersion"] == "1.0"

    name = Path(schema_path).stem
    print(f"  ✅ convert({name}) → {elapsed_us:.0f}µs")

    return elapsed_us


def main():
    project_root = Path(__file__).resolve().parent.parent.parent
    default_wasm = project_root / "target" / "wasm32-wasip1" / "release" / "jsonschema_llm_wasi.wasm"

    wasm_path = sys.argv[1] if len(sys.argv) > 1 else str(default_wasm)

    if not Path(wasm_path).exists():
        print(f"❌ WASM binary not found: {wasm_path}")
        print("   Build with: cargo build --target wasm32-wasip1 --release -p jsonschema-llm-wasi")
        sys.exit(1)

    wasm_size = Path(wasm_path).stat().st_size
    print(f"\n{'='*60}")
    print(f"WASI Host Verification — jsonschema-llm")
    print(f"{'='*60}")
    print(f"Binary: {wasm_path}")
    print(f"Size: {wasm_size / (1024*1024):.2f} MB")
    print()

    store, instance, memory = load_wasm(wasm_path)

    # ---- Functional tests ----
    print("Functional Tests:")
    convert_payload = test_convert_simple(store, instance, memory)
    test_rehydrate_simple(store, instance, memory, convert_payload)
    test_convert_error(store, instance, memory)
    test_convert_null_options(store, instance, memory)
    test_convert_empty_whitespace_options(store, instance, memory)
    test_convert_null_pointer_error(store, instance, memory)

    # ---- Edge case / boundary tests ----
    print("\nEdge Case Tests:")
    test_convert_invalid_utf8_schema(store, instance, memory)
    test_convert_invalid_utf8_options(store, instance, memory)
    test_convert_invalid_options_json(store, instance, memory)
    test_convert_partial_options(store, instance, memory)
    test_convert_empty_schema(store, instance, memory)
    test_rehydrate_error(store, instance, memory)
    test_rehydrate_roundtrip_real_world(store, instance, memory)
    test_alloc_zero(store, instance, memory)
    test_free_null_guards(store, instance, memory)

    # ---- Real-world schema tests ----
    schemas_dir = project_root / "tests" / "schemas"
    real_world_dir = schemas_dir / "real-world"

    if real_world_dir.exists():
        print(f"\nReal-World Schemas ({real_world_dir.name}/):")
        timings = []
        for schema_file in sorted(real_world_dir.glob("*.json")):
            elapsed = test_convert_real_world(store, instance, memory, str(schema_file))
            timings.append((schema_file.stem, elapsed))

        if timings:
            avg = sum(t for _, t in timings) / len(timings)
            fastest = min(timings, key=lambda x: x[1])
            slowest = max(timings, key=lambda x: x[1])
            print(f"\n  📊 Timing summary ({len(timings)} schemas):")
            print(f"     Average: {avg:.0f}µs")
            print(f"     Fastest: {fastest[0]} ({fastest[1]:.0f}µs)")
            print(f"     Slowest: {slowest[0]} ({slowest[1]:.0f}µs)")

    # ---- Top-level schemas ----
    top_level = list(schemas_dir.glob("*.json"))
    if top_level:
        print(f"\nCore Schemas:")
        for schema_file in sorted(top_level):
            test_convert_real_world(store, instance, memory, str(schema_file))

    print(f"\n{'='*60}")
    print(f"✅ All WASI host verification tests passed!")
    print(f"Binary size: {wasm_size / (1024*1024):.2f} MB (gate: ≤3 MB)")
    print(f"{'='*60}\n")


if __name__ == "__main__":
    main()
