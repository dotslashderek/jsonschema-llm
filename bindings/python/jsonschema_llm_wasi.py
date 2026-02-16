"""
WASI-backed wrapper for jsonschema-llm.

Uses wasmtime-py to load the universal WASI binary and exposes
convert() and rehydrate() as Python functions.

Concurrency: Each Engine owns its own Store. NOT thread-safe.
"""

import json
import os
import struct
from typing import Any, Optional

import wasmtime


_DEFAULT_WASM_PATH = os.path.join(
    os.path.dirname(__file__),
    "..", "..",
    "target", "wasm32-wasip1", "release", "jsonschema_llm_wasi.wasm",
)

JSL_RESULT_SIZE = 12  # 3 × u32 (LE)
EXPECTED_ABI_VERSION = 1
STATUS_OK = 0
STATUS_ERROR = 1


class JslError(Exception):
    """Structured error from the WASI binary."""

    def __init__(self, code: str, message: str, path: str = ""):
        self.code = code
        self.message = message
        self.path = path
        super().__init__(f"jsl error [{code}]{f' at {path}' if path else ''}: {message}")


class Engine:
    """
    WASI-backed jsonschema-llm engine.

    Usage::

        with Engine() as eng:
            result = eng.convert(schema)
            rehydrated = eng.rehydrate(data, result["codec"], schema)
    """

    def __init__(self, wasm_path: Optional[str] = None):
        path = wasm_path or os.environ.get("JSL_WASM_PATH", _DEFAULT_WASM_PATH)
        self._engine = wasmtime.Engine()
        self._module = wasmtime.Module.from_file(self._engine, path)
        self._linker = wasmtime.Linker(self._engine)
        self._linker.define_wasi()
        self._abi_verified = False

    def __enter__(self):
        return self

    def __exit__(self, *_):
        pass  # No persistent resources to clean up

    def convert(self, schema: Any, options: Optional[dict] = None) -> dict:
        """Convert a JSON Schema to LLM-compatible structured output schema."""
        schema_json = json.dumps(schema)
        # Normalize snake_case keys to kebab-case for WASI binary compatibility
        normalized = {}
        if options:
            for key, value in options.items():
                normalized[key.replace("_", "-")] = value
        opts_json = json.dumps(normalized)
        return self._call_jsl("jsl_convert", schema_json, opts_json)

    def rehydrate(self, data: Any, codec: Any, schema: Any) -> dict:
        """Rehydrate LLM output back to original schema shape."""
        data_json = json.dumps(data)
        codec_json = json.dumps(codec)
        schema_json = json.dumps(schema)
        return self._call_jsl("jsl_rehydrate", data_json, codec_json, schema_json)

    def _call_jsl(self, func_name: str, *json_args: str) -> dict:
        """Execute a WASI export following the JslResult protocol."""
        # Fresh store + instance per call (WASI modules are single-use)
        store = wasmtime.Store(self._engine)
        store.set_wasi(wasmtime.WasiConfig())
        instance = self._linker.instantiate(store, self._module)

        # ABI version handshake (once per Engine lifetime)
        if not self._abi_verified:
            exports = instance.exports(store)
            try:
                abi_fn = exports["jsl_abi_version"]
            except KeyError:
                raise RuntimeError(
                    "Incompatible WASM module: missing required 'jsl_abi_version' export"
                )
            version = abi_fn(store)
            if version != EXPECTED_ABI_VERSION:
                raise RuntimeError(
                    f"ABI version mismatch: binary={version}, expected={EXPECTED_ABI_VERSION}"
                )
            self._abi_verified = True

        memory = instance.exports(store)["memory"]
        jsl_alloc = instance.exports(store)["jsl_alloc"]
        jsl_free = instance.exports(store)["jsl_free"]
        jsl_result_free = instance.exports(store)["jsl_result_free"]
        func = instance.exports(store)[func_name]

        # Allocate and write each argument
        allocs = []
        flat_args = []
        for arg in json_args:
            data = arg.encode("utf-8")
            ptr = jsl_alloc(store, len(data))
            if ptr == 0 and len(data) > 0:
                raise RuntimeError(f"jsl_alloc returned null for {len(data)} bytes")
            memory.write(store, data, ptr)
            allocs.append((ptr, len(data)))
            flat_args.extend([ptr, len(data)])

        # Call, read, and parse — with guaranteed cleanup
        result_ptr = 0
        try:
            result_ptr = func(store, *flat_args)
            if result_ptr == 0:
                raise RuntimeError(f"{func_name} returned null result pointer")

            # Read JslResult (12 bytes: 3 × LE u32)
            result_bytes = memory.read(store, result_ptr, result_ptr + JSL_RESULT_SIZE)
            status, payload_ptr, payload_len = struct.unpack("<III", result_bytes)

            # Validate payload bounds
            mem_size = memory.data_len(store)
            if payload_ptr + payload_len > mem_size:
                raise RuntimeError(
                    f"payload out of bounds: ptr={payload_ptr} len={payload_len} memSize={mem_size}"
                )

            # Read and copy payload
            payload_bytes = memory.read(store, payload_ptr, payload_ptr + payload_len)
            payload = json.loads(payload_bytes.decode("utf-8"))

            if status == STATUS_ERROR:
                raise JslError(
                    code=payload.get("code", "unknown"),
                    message=payload.get("message", "unknown error"),
                    path=payload.get("path", ""),
                )

            return payload
        finally:
            if result_ptr != 0:
                jsl_result_free(store, result_ptr)
            for ptr, length in allocs:
                jsl_free(store, ptr, length)
