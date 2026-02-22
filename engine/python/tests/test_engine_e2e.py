"""E2E integration tests for the Python LlmRoundtripEngine.

These tests load the REAL jsonschema_llm_wasi.wasm binary and exercise
the full convert → rehydrate pipeline through wasmtime. They verify that
the engine layer correctly handles the JslResult ABI protocol end-to-end.

Acceptance criteria (Issue #209):
- At least one smoke test executes against the real WASM module
- Tests skip gracefully if the WASM binary is not built

Written RED before implementation (TDD gate).
"""

from __future__ import annotations

import json
import os
from pathlib import Path

import pytest


# WASM binary path: env var → repo-relative fallback
def _find_wasm_path() -> Path:
    env = os.environ.get("JSONSCHEMA_LLM_WASM_PATH")
    if env:
        p = Path(env)
        if p.is_file():
            return p
    return (
        Path(__file__).resolve().parent.parent.parent.parent
        / "target"
        / "wasm32-wasip1"
        / "release"
        / "jsonschema_llm_wasi.wasm"
    )


_WASM_PATH = _find_wasm_path()
_WASM_EXISTS = _WASM_PATH.is_file()

e2e = pytest.mark.skipif(
    not _WASM_EXISTS,
    reason=f"WASM binary not found at {_WASM_PATH}. Build with: make build-wasi",
)

# Mark all tests in this module as e2e
pytestmark = [pytest.mark.e2e, e2e]

# ── Schemas ──────────────────────────────────────────────────────────────────

PERSON_SCHEMA = json.dumps(
    {
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer", "minimum": 0},
        },
        "required": ["name", "age"],
    }
)


# ── Stub Transport ───────────────────────────────────────────────────────────


class StubTransport:
    """Minimal transport that returns a deterministic OpenAI-shaped response."""

    def __init__(self, content_json: str) -> None:
        self._response = json.dumps(
            {"choices": [{"message": {"content": content_json}}]}
        )

    def execute(self, request) -> str:  # noqa: ANN001
        return self._response


# ── Tests ────────────────────────────────────────────────────────────────────


class TestConvertRoundtrip:
    """Verify that _call_wasi("jsl_convert") works end-to-end with real WASM."""

    def test_convert_returns_expected_keys(self) -> None:
        from jsonschema_llm_engine.engine import LlmRoundtripEngine

        engine = LlmRoundtripEngine(wasm_path=str(_WASM_PATH))
        result = engine._call_wasi("jsl_convert", PERSON_SCHEMA, "{}")

        assert "apiVersion" in result, f"Missing apiVersion: {result}"
        assert result["apiVersion"] == "1.0"
        assert "schema" in result, f"Missing schema key: {result}"
        assert "codec" in result, f"Missing codec key: {result}"
        assert isinstance(result["schema"], dict)

    def test_convert_schema_has_properties(self) -> None:
        from jsonschema_llm_engine.engine import LlmRoundtripEngine

        engine = LlmRoundtripEngine(wasm_path=str(_WASM_PATH))
        result = engine._call_wasi("jsl_convert", PERSON_SCHEMA, "{}")
        schema = result["schema"]

        # Converted schema should still have object structure
        assert schema.get("type") == "object" or "properties" in schema


class TestRehydrateRoundtrip:
    """Verify convert → rehydrate round-trip with real WASM."""

    def test_rehydrate_recovers_original_data(self) -> None:
        from jsonschema_llm_engine.engine import LlmRoundtripEngine

        engine = LlmRoundtripEngine(wasm_path=str(_WASM_PATH))

        # Step 1: Convert
        convert_result = engine._call_wasi("jsl_convert", PERSON_SCHEMA, "{}")
        codec_json = json.dumps(convert_result["codec"])

        # Step 2: Rehydrate with known data
        data_json = json.dumps({"name": "Ada", "age": 36})
        rehydrate_result = engine._call_wasi(
            "jsl_rehydrate", data_json, codec_json, PERSON_SCHEMA
        )

        assert "apiVersion" in rehydrate_result
        assert "data" in rehydrate_result
        assert rehydrate_result["data"]["name"] == "Ada"
        assert rehydrate_result["data"]["age"] == 36


class TestConvertErrorPropagates:
    """Verify that WASM errors propagate as SchemaConversionError."""

    def test_invalid_json_raises_schema_conversion_error(self) -> None:
        from jsonschema_llm_engine.engine import LlmRoundtripEngine
        from jsonschema_llm_engine.exceptions import SchemaConversionError

        engine = LlmRoundtripEngine(wasm_path=str(_WASM_PATH))

        with pytest.raises(SchemaConversionError) as exc_info:
            engine._call_wasi("jsl_convert", "NOT VALID JSON", "{}")

        # Error should include structured error code
        assert "json_parse_error" in str(exc_info.value)


class TestGenerateFullRoundtrip:
    """Full engine.generate() with real WASM + stub transport."""

    def test_generate_returns_valid_roundtrip_result(self) -> None:
        from jsonschema_llm_engine.engine import LlmRoundtripEngine
        from jsonschema_llm_engine.formatters.chat_completions import (
            ChatCompletionsFormatter,
        )
        from jsonschema_llm_engine.types import ProviderConfig, RoundtripResult

        engine = LlmRoundtripEngine(wasm_path=str(_WASM_PATH))
        formatter = ChatCompletionsFormatter()
        config = ProviderConfig(
            url="https://api.openai.com/v1/chat/completions",
            model="gpt-4o",
            headers={"Authorization": "Bearer test-key"},
        )
        transport = StubTransport('{"name": "Ada", "age": 36}')

        result = engine.generate(
            schema_json=PERSON_SCHEMA,
            prompt="Generate a person",
            formatter=formatter,
            config=config,
            transport=transport,
        )

        assert isinstance(result, RoundtripResult)
        assert result.data["name"] == "Ada"
        assert result.data["age"] == 36
        assert result.is_valid is True
        assert result.validation_errors == []
