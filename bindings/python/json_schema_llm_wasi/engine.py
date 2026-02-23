"""
WASI-backed SchemaLlmEngine for json-schema-llm.

Uses wasmtime-py to load the universal WASI binary and exposes
typed schema conversion and rehydration operations.

Concurrency: Each SchemaLlmEngine owns its own Store. NOT thread-safe.
"""

import json
import os
import struct
from typing import Any, Optional

import wasmtime

from json_schema_llm_wasi.types import (
    ConvertAllComponentsResult,
    ConvertOptions,
    ConvertResult,
    ExtractComponentResult,
    ListComponentsResult,
    RehydrateResult,
)

_DEFAULT_WASM_PATH = os.path.join(
    os.path.dirname(__file__),
    "..", "..", "..",
    "target", "wasm32-wasip1", "release", "json_schema_llm_wasi.wasm",
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


class SchemaLlmEngine:
    """WASI-backed json-schema-llm engine with typed results.

    Usage::

        with SchemaLlmEngine() as engine:
            result = engine.convert(schema, ConvertOptions(target="openai-strict"))
            optimized = result.schema
            codec = result.codec

            rehydrated = engine.rehydrate(data, codec, schema)
            restored = rehydrated.data
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

    def convert(
        self, schema: Any, options: Optional[ConvertOptions] = None
    ) -> ConvertResult:
        """Convert a JSON Schema to LLM-compatible structured output schema."""
        schema_json = json.dumps(schema)
        opts_dict = options.to_dict() if options else {}
        opts_json = json.dumps(opts_dict)
        raw = self._call_jsl("jsl_convert", schema_json, opts_json)
        return ConvertResult.from_dict(raw)

    def rehydrate(self, data: Any, codec: Any, schema: Any) -> RehydrateResult:
        """Rehydrate LLM output back to original schema shape."""
        data_json = json.dumps(data)
        codec_json = json.dumps(codec)
        schema_json = json.dumps(schema)
        raw = self._call_jsl("jsl_rehydrate", data_json, codec_json, schema_json)
        return RehydrateResult.from_dict(raw)

    def list_components(self, schema: Any) -> ListComponentsResult:
        """List all extractable component JSON Pointers in a schema."""
        schema_json = json.dumps(schema)
        raw = self._call_jsl("jsl_list_components", schema_json)
        return ListComponentsResult.from_dict(raw)

    def extract_component(
        self, schema: Any, pointer: str, options: Optional[dict] = None
    ) -> ExtractComponentResult:
        """Extract a single component from a schema by JSON Pointer."""
        schema_json = json.dumps(schema)
        opts_json = json.dumps(options or {})
        raw = self._call_jsl("jsl_extract_component", schema_json, pointer, opts_json)
        return ExtractComponentResult.from_dict(raw)

    def convert_all_components(
        self,
        schema: Any,
        convert_options: Optional[ConvertOptions] = None,
        extract_options: Optional[dict] = None,
    ) -> ConvertAllComponentsResult:
        """Convert a schema and all its discoverable components in one call."""
        schema_json = json.dumps(schema)
        conv_dict = convert_options.to_dict() if convert_options else {}
        conv_opts_json = json.dumps(conv_dict)
        ext_opts_json = json.dumps(extract_options or {})
        raw = self._call_jsl(
            "jsl_convert_all_components", schema_json, conv_opts_json, ext_opts_json
        )
        return ConvertAllComponentsResult.from_dict(raw)

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
