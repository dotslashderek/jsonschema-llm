"""LLM Roundtrip Engine — orchestrates the full convert → format → call → rehydrate → validate pipeline."""

from __future__ import annotations

import json
from typing import Any, Optional

import wasmtime

from json_schema_llm_engine.exceptions import (
    RehydrationError,
    ResponseParsingError,
    SchemaConversionError,
)
from json_schema_llm_engine.formatter import ProviderFormatter
from json_schema_llm_engine.transport import LlmTransport
from json_schema_llm_engine.types import LlmRequest, ProviderConfig, RoundtripResult

# ABI version expected from the WASI module
_EXPECTED_ABI_VERSION = 1
_RESULT_SIZE = 12  # 3 × u32 (status, ptr, len)
_STATUS_OK = 0
_STATUS_ERROR = 1


class LlmRoundtripEngine:
    """Orchestrates the full LLM roundtrip.

    Lifecycle: convert schema → format request → call LLM → extract content →
    rehydrate output → validate against original schema.

    The wasmtime Engine and compiled Module are cached at init time.
    A fresh Store + Instance is created per call (WASI modules are single-use).

    Args:
        wasm_path: Path to the json-schema-llm WASI binary. If None, uses
                   the JSON_SCHEMA_LLM_WASM_PATH environment variable or
                   falls back to importlib.resources.
    """

    def __init__(self, wasm_path: Optional[str] = None) -> None:
        wasm_bytes = _resolve_wasm_bytes(wasm_path)
        self._engine = wasmtime.Engine()
        self._module = wasmtime.Module(self._engine, wasm_bytes)

    def generate(
        self,
        schema_json: str,
        prompt: str,
        formatter: ProviderFormatter,
        config: ProviderConfig,
        transport: LlmTransport,
    ) -> RoundtripResult:
        """Full roundtrip: convert → format → call LLM → rehydrate → validate.

        Args:
            schema_json: The original JSON Schema as a string.
            prompt: The user's natural language prompt.
            formatter: Provider-specific request formatter.
            config: Provider endpoint configuration.
            transport: Consumer-provided HTTP transport.

        Returns:
            RoundtripResult with rehydrated data and validation status.
        """
        # Step 1: Convert schema to LLM-compatible form
        convert_result = self._call_wasi("jsl_convert", schema_json, "{}")
        llm_schema = convert_result.get("schema", {})
        codec = convert_result.get("codec", {})

        return self.generate_with_preconverted(
            schema_json=schema_json,
            codec_json=json.dumps(codec),
            llm_schema=llm_schema,
            prompt=prompt,
            formatter=formatter,
            config=config,
            transport=transport,
        )

    def generate_with_preconverted(
        self,
        schema_json: str,
        codec_json: str,
        llm_schema: Any,
        prompt: str,
        formatter: ProviderFormatter,
        config: ProviderConfig,
        transport: LlmTransport,
    ) -> RoundtripResult:
        """Roundtrip with pre-converted schema (skips the convert step).

        Use when you have pre-built schema/codec from gen-sdk.

        Args:
            schema_json: The original JSON Schema as a string.
            codec_json: The codec (rehydration map) as a string.
            llm_schema: The LLM-compatible schema (already converted).
            prompt: The user's natural language prompt.
            formatter: Provider-specific request formatter.
            config: Provider endpoint configuration.
            transport: Consumer-provided HTTP transport.

        Returns:
            RoundtripResult with rehydrated data and validation status.
        """
        # Step 2: Format the request for the provider
        request: LlmRequest = formatter.format(prompt, llm_schema, config)

        # Step 3: Call the LLM via consumer transport
        raw_response = transport.execute(request)

        # Step 4: Extract content from the response
        try:
            content = formatter.extract_content(raw_response)
        except ResponseParsingError:
            raise
        except Exception as e:
            raise ResponseParsingError(f"Failed to extract content: {e}") from e

        # Step 5: Rehydrate the output
        rehydrate_result = self._call_wasi(
            "jsl_rehydrate", content, codec_json, schema_json
        )

        rehydrated_data = rehydrate_result.get("data", {})
        warnings = rehydrate_result.get("warnings", [])

        # Step 6: Validate against original schema
        validation_errors = self._validate(rehydrated_data, schema_json)

        return RoundtripResult(
            data=rehydrated_data,
            raw_llm_response=json.loads(raw_response),
            warnings=warnings,
            validation_errors=validation_errors,
        )

    def _validate(self, data: Any, schema_json: str) -> list[str]:
        """Validate data against JSON Schema using the jsonschema library."""
        try:
            import jsonschema
            import jsonschema.exceptions

            schema = json.loads(schema_json)
            jsonschema.Draft202012Validator.check_schema(schema)
            validator = jsonschema.Draft202012Validator(schema)
            return [str(e.message) for e in validator.iter_errors(data)]
        except ImportError:
            return []
        except (
            jsonschema.exceptions.SchemaError,
            jsonschema.exceptions.UnknownType,
        ) as e:
            msg = e.message if hasattr(e, "message") else str(e)
            return [f"Schema validation error: {msg}"]

    # ─── WASI Internals ─────────────────────────────────────────────────

    def _call_wasi(self, func_name: str, *json_args: str) -> Any:
        """Execute a WASI export following the JslResult protocol."""
        import struct

        store = wasmtime.Store(self._engine)
        store.set_wasi(wasmtime.WasiConfig())
        linker = wasmtime.Linker(self._engine)
        linker.define_wasi()
        instance = linker.instantiate(store, self._module)

        exports = instance.exports(store)
        memory = exports["memory"]
        jsl_alloc = exports["jsl_alloc"]
        jsl_free = exports["jsl_free"]
        jsl_result_free = exports["jsl_result_free"]
        func = exports[func_name]

        # ABI handshake
        abi_fn = exports["jsl_abi_version"]
        version = abi_fn(store)
        if version != _EXPECTED_ABI_VERSION:
            raise SchemaConversionError(
                f"ABI version mismatch: binary={version}, expected={_EXPECTED_ABI_VERSION}"
            )

        allocs: list[tuple[int, int]] = []
        flat_args: list[int] = []
        result_ptr = 0

        try:
            for arg in json_args:
                data = arg.encode("utf-8")
                ptr = jsl_alloc(store, len(data))
                if ptr == 0 and len(data) > 0:
                    raise SchemaConversionError(
                        f"jsl_alloc returned null for {len(data)} bytes"
                    )
                memory.write(store, data, ptr)
                allocs.append((ptr, len(data)))
                flat_args.extend([ptr, len(data)])

            result_ptr = func(store, *flat_args)
            if result_ptr == 0:
                raise SchemaConversionError(f"{func_name} returned null result pointer")

            # Read result struct (12 bytes: 3 × LE u32)
            result_bytes = memory.read(store, result_ptr, result_ptr + _RESULT_SIZE)
            status, payload_ptr, payload_len = struct.unpack("<III", result_bytes)

            # Bounds check before payload read
            mem_size = memory.data_len(store)
            if payload_ptr + payload_len > mem_size:
                raise SchemaConversionError(
                    f"payload out of bounds: ptr={payload_ptr} len={payload_len} memSize={mem_size}"
                )

            payload_bytes = memory.read(store, payload_ptr, payload_ptr + payload_len)
            payload_str = payload_bytes.decode("utf-8")
            payload = json.loads(payload_str)

            if status == _STATUS_ERROR:
                error_msg = (
                    payload.get("message", "unknown error")
                    if isinstance(payload, dict)
                    else str(payload)
                )
                error_code = (
                    payload.get("code", "unknown")
                    if isinstance(payload, dict)
                    else "unknown"
                )
                if "rehydrat" in func_name.lower():
                    raise RehydrationError(f"[{error_code}] {error_msg}")
                raise SchemaConversionError(f"[{error_code}] {error_msg}")

            return payload
        finally:
            if result_ptr != 0:
                jsl_result_free(store, result_ptr)
            for ptr, length in allocs:
                jsl_free(store, ptr, length)


def _resolve_wasm_bytes(explicit: str | None) -> bytes:
    """Resolve WASM binary: explicit path → env var → importlib.resources."""
    import os
    from pathlib import Path

    # Tier 1: Explicit path
    if explicit:
        p = Path(explicit)
        if p.is_file():
            return p.read_bytes()
        raise FileNotFoundError(f"Explicit WASM path not found: {p}")

    # Tier 2: Environment variable
    env = os.environ.get("JSON_SCHEMA_LLM_WASM_PATH")
    if env:
        p = Path(env)
        if p.is_file():
            return p.read_bytes()

    # Tier 3: Repo-relative fallback (dev/CI)
    repo_path = (
        Path(__file__).resolve().parent.parent.parent.parent
        / "target"
        / "wasm32-wasip1"
        / "release"
        / "json_schema_llm_wasi.wasm"
    )
    if repo_path.is_file():
        return repo_path.read_bytes()

    raise FileNotFoundError(
        "WASM binary not found. Set JSON_SCHEMA_LLM_WASM_PATH or build the WASI target."
    )
