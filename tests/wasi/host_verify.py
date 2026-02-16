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
STATUS_PANIC = 2

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
