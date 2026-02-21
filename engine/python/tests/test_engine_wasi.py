"""Acceptance tests for Issue #207: WASI ABI mismatch & validation swallowing.

These tests verify the 3 bug fixes:
1. generate() passes 2 JSON args to _call_wasi (schema + options)
2. generate() reads the 'schema' key (not 'data') from convert result
3. _validate() surfaces errors instead of swallowing them

Written RED before implementation (TDD gate).
"""

import json
from unittest.mock import MagicMock, patch

import pytest


# ── Bug 1: WASI ABI parameter mismatch ──────────────────────────────────────


class TestGeneratePassesOptionsToConvert:
    """generate() must call _call_wasi("jsl_convert", schema_json, "{}") — 2 JSON args."""

    def test_generate_passes_two_json_args_to_convert(self):
        from jsonschema_llm_engine.engine import LlmRoundtripEngine
        from jsonschema_llm_engine.types import ProviderConfig

        schema_json = json.dumps(
            {"type": "object", "properties": {"name": {"type": "string"}}}
        )

        # Create engine with mocked WASM loading
        with patch(
            "jsonschema_llm_engine.engine._resolve_wasm_bytes", return_value=b"fake"
        ):
            with patch("wasmtime.Engine"):
                with patch("wasmtime.Module"):
                    engine = LlmRoundtripEngine.__new__(LlmRoundtripEngine)
                    engine._engine = MagicMock()
                    engine._module = MagicMock()

        # Mock _call_wasi to capture calls
        convert_result = {
            "apiVersion": "1.0",
            "schema": {"type": "object", "properties": {"name": {"type": "string"}}},
            "codec": {},
        }
        rehydrate_result = {
            "apiVersion": "1.0",
            "data": {"name": "Alice"},
            "warnings": [],
        }

        call_log = []

        def mock_call_wasi(func_name, *args):
            call_log.append((func_name, args))
            if func_name == "jsl_convert":
                return convert_result
            if func_name == "jsl_rehydrate":
                return rehydrate_result
            return {}

        engine._call_wasi = mock_call_wasi

        # Mock formatter + transport
        formatter = MagicMock()
        formatter.format.return_value = MagicMock(
            url="http://test", headers={}, body="{}"
        )
        formatter.extract_content.return_value = '{"name": "Alice"}'

        transport = MagicMock()
        transport.execute.return_value = '{"choices": []}'

        config = ProviderConfig(url="http://test", model="test-model")

        engine.generate(schema_json, "test prompt", formatter, config, transport)

        # Assert: jsl_convert was called with 2 JSON args (schema + options)
        convert_call = [c for c in call_log if c[0] == "jsl_convert"]
        assert len(convert_call) == 1, (
            f"Expected 1 jsl_convert call, got {len(convert_call)}"
        )
        args = convert_call[0][1]
        assert len(args) == 2, (
            f"Expected 2 JSON args to jsl_convert, got {len(args)}: {args}"
        )
        assert args[0] == schema_json
        assert args[1] == "{}"


# ── Bug 2: Wrong result key ─────────────────────────────────────────────────


class TestGenerateReadsSchemaKey:
    """generate() must read the 'schema' key, not 'data', from convert result."""

    def test_generate_uses_schema_key_from_convert_result(self):
        from jsonschema_llm_engine.engine import LlmRoundtripEngine
        from jsonschema_llm_engine.types import ProviderConfig

        schema_json = json.dumps(
            {"type": "object", "properties": {"name": {"type": "string"}}}
        )
        expected_llm_schema = {
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "additionalProperties": False,
        }

        # Create engine
        engine = LlmRoundtripEngine.__new__(LlmRoundtripEngine)
        engine._engine = MagicMock()
        engine._module = MagicMock()

        convert_result = {
            "apiVersion": "1.0",
            "schema": expected_llm_schema,
            "codec": {"mapping": {}},
            # Intentionally no "data" key — if code looks for "data", it gets {}
        }
        rehydrate_result = {
            "apiVersion": "1.0",
            "data": {"name": "Alice"},
            "warnings": [],
        }

        def mock_call_wasi(func_name, *args):
            if func_name == "jsl_convert":
                return convert_result
            if func_name == "jsl_rehydrate":
                return rehydrate_result
            return {}

        engine._call_wasi = mock_call_wasi

        # Capture what schema gets passed to formatter.format
        formatter = MagicMock()
        formatter.format.return_value = MagicMock(
            url="http://test", headers={}, body="{}"
        )
        formatter.extract_content.return_value = '{"name": "Alice"}'

        transport = MagicMock()
        transport.execute.return_value = '{"choices": []}'

        config = ProviderConfig(url="http://test", model="test-model")

        engine.generate(schema_json, "test prompt", formatter, config, transport)

        # The formatter should receive the schema from convert_result["schema"],
        # NOT an empty dict from convert_result.get("data", {})
        format_call_args = formatter.format.call_args
        actual_llm_schema = format_call_args[0][1]  # second positional arg
        assert actual_llm_schema == expected_llm_schema, (
            f"Expected formatter to receive schema={expected_llm_schema}, got {actual_llm_schema}"
        )


# ── Bug 3: Silent validation swallowing ─────────────────────────────────────


class TestValidateSurfacesErrors:
    """_validate() must surface validation errors, not silently return []."""

    def test_validate_returns_errors_for_invalid_data(self):
        from jsonschema_llm_engine.engine import LlmRoundtripEngine

        engine = LlmRoundtripEngine.__new__(LlmRoundtripEngine)

        schema_json = json.dumps(
            {
                "type": "object",
                "properties": {"name": {"type": "string"}, "age": {"type": "integer"}},
                "required": ["name", "age"],
            }
        )

        # Missing required 'age' field
        data = {"name": "Alice"}
        errors = engine._validate(data, schema_json)

        assert len(errors) > 0, (
            "Expected validation errors for missing required field, got []"
        )
        assert any("age" in e for e in errors), (
            f"Expected 'age' in validation errors: {errors}"
        )

    def test_validate_does_not_swallow_json_decode_error(self):
        from jsonschema_llm_engine.engine import LlmRoundtripEngine

        engine = LlmRoundtripEngine.__new__(LlmRoundtripEngine)

        # Malformed JSON should propagate, not be swallowed
        with pytest.raises(json.JSONDecodeError):
            engine._validate({"name": "Alice"}, "NOT VALID JSON")

    def test_validate_graceful_on_missing_jsonschema(self):
        from jsonschema_llm_engine.engine import LlmRoundtripEngine

        engine = LlmRoundtripEngine.__new__(LlmRoundtripEngine)

        schema_json = json.dumps({"type": "object"})

        # Simulate jsonschema not installed
        with patch.dict("sys.modules", {"jsonschema": None}):
            with patch(
                "builtins.__import__",
                side_effect=ImportError("No module named 'jsonschema'"),
            ):
                errors = engine._validate({"key": "value"}, schema_json)

        assert errors == [], f"Expected [] when jsonschema is missing, got {errors}"

    def test_validate_reports_schema_error(self):
        """SchemaError should appear as a synthetic validation error, not crash or be swallowed."""
        from jsonschema_llm_engine.engine import LlmRoundtripEngine

        engine = LlmRoundtripEngine.__new__(LlmRoundtripEngine)

        # A schema with an invalid value for 'type' will trigger SchemaError
        # when Draft202012Validator validates data against it
        schema_json = json.dumps(
            {
                "type": "invalid_type_value",
            }
        )

        errors = engine._validate({"name": "Alice"}, schema_json)

        # Should NOT be empty — the schema error should be surfaced
        assert len(errors) > 0, "Expected schema error to be reported, got []"
