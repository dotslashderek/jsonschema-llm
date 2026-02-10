"""Acceptance tests for jsonschema-llm Python bindings.

Validates all acceptance criteria from issues #60, #61, #62.
Run: python -m pytest tests/test_acceptance.py -v
"""

import pytest
from jsonschema_llm import convert, rehydrate, JsonSchemaLlmError


# ---------------------------------------------------------------------------
# Issue #60: Module scaffold
# ---------------------------------------------------------------------------

class TestModuleScaffold:
    """Verify PyO3 module loads and exports are available."""

    def test_import_module(self):
        import jsonschema_llm
        assert hasattr(jsonschema_llm, "convert")
        assert hasattr(jsonschema_llm, "rehydrate")
        assert hasattr(jsonschema_llm, "JsonSchemaLlmError")

    def test_convert_is_callable(self):
        assert callable(convert)

    def test_rehydrate_is_callable(self):
        assert callable(rehydrate)


# ---------------------------------------------------------------------------
# Issue #61: convert/rehydrate exports
# ---------------------------------------------------------------------------

BASIC_SCHEMA = {
    "type": "object",
    "properties": {
        "name": {"type": "string"},
        "age": {"type": "integer"},
    },
    "required": ["name"],
}


class TestConvert:
    """Verify convert() accepts dict and returns correct structure."""

    def test_convert_returns_dict(self):
        result = convert(BASIC_SCHEMA)
        assert isinstance(result, dict)

    def test_convert_has_api_version(self):
        result = convert(BASIC_SCHEMA)
        assert result["api_version"] == "1.0"

    def test_convert_has_schema(self):
        result = convert(BASIC_SCHEMA)
        assert "schema" in result
        assert isinstance(result["schema"], dict)

    def test_convert_has_codec(self):
        result = convert(BASIC_SCHEMA)
        assert "codec" in result
        assert isinstance(result["codec"], dict)

    def test_convert_with_options(self):
        result = convert(BASIC_SCHEMA, {"target": "openai-strict", "max_depth": 10})
        assert result["api_version"] == "1.0"
        assert "schema" in result
        assert "codec" in result

    def test_convert_with_none_options(self):
        result = convert(BASIC_SCHEMA, None)
        assert result["api_version"] == "1.0"

    def test_convert_without_options_uses_defaults(self):
        result = convert(BASIC_SCHEMA)
        assert result["api_version"] == "1.0"

    def test_convert_gemini_target(self):
        result = convert(BASIC_SCHEMA, {"target": "gemini"})
        assert "schema" in result

    def test_convert_claude_target(self):
        result = convert(BASIC_SCHEMA, {"target": "claude"})
        assert "schema" in result


class TestRehydrate:
    """Verify rehydrate() accepts dict and returns correct structure."""

    def test_rehydrate_returns_dict(self):
        cr = convert(BASIC_SCHEMA)
        result = rehydrate({"name": "Ada", "age": 36}, cr["codec"])
        assert isinstance(result, dict)

    def test_rehydrate_has_api_version(self):
        cr = convert(BASIC_SCHEMA)
        result = rehydrate({"name": "Ada"}, cr["codec"])
        assert result["api_version"] == "1.0"

    def test_rehydrate_has_data(self):
        cr = convert(BASIC_SCHEMA)
        result = rehydrate({"name": "Ada", "age": 36}, cr["codec"])
        assert "data" in result
        assert result["data"]["name"] == "Ada"
        assert result["data"]["age"] == 36

    def test_rehydrate_has_warnings(self):
        cr = convert(BASIC_SCHEMA)
        result = rehydrate({"name": "Ada"}, cr["codec"])
        assert "warnings" in result
        assert isinstance(result["warnings"], list)


# ---------------------------------------------------------------------------
# Issue #62: ConvertError → Python exceptions
# ---------------------------------------------------------------------------

class TestErrorHandling:
    """Verify ConvertError maps to JsonSchemaLlmError with structured attrs."""

    def test_unresolvable_ref_raises(self):
        with pytest.raises(JsonSchemaLlmError):
            convert({"$ref": "#/$defs/NonExistent"})

    def test_error_has_code_attr(self):
        with pytest.raises(JsonSchemaLlmError) as exc_info:
            convert({"$ref": "#/$defs/NonExistent"})
        assert exc_info.value.code == "unresolvable_ref"

    def test_error_has_message_attr(self):
        with pytest.raises(JsonSchemaLlmError) as exc_info:
            convert({"$ref": "#/$defs/NonExistent"})
        # Message wording is not guaranteed stable; just verify it is a non-empty string
        assert isinstance(exc_info.value.message, str)
        assert exc_info.value.message

    def test_error_has_path_attr(self):
        with pytest.raises(JsonSchemaLlmError) as exc_info:
            convert({"$ref": "#/$defs/NonExistent"})
        assert exc_info.value.path is not None

    def test_error_is_exception_subclass(self):
        assert issubclass(JsonSchemaLlmError, Exception)

    def test_error_catchable_as_exception(self):
        """JsonSchemaLlmError can be caught as a generic Exception."""
        with pytest.raises(Exception):
            convert({"$ref": "#/$defs/NonExistent"})


# ---------------------------------------------------------------------------
# Snake_case API convention
# ---------------------------------------------------------------------------

class TestSnakeCaseApi:
    """Verify Python API uses snake_case throughout."""

    def test_function_names(self):
        import jsonschema_llm
        # Functions should be snake_case
        assert hasattr(jsonschema_llm, "convert")
        assert hasattr(jsonschema_llm, "rehydrate")
        # No camelCase
        assert not hasattr(jsonschema_llm, "convertSchema")

    def test_result_keys_are_snake_case(self):
        result = convert(BASIC_SCHEMA)
        assert "api_version" in result
        # Not camelCase
        assert "apiVersion" not in result

    def test_options_accept_snake_case(self):
        # max_depth, recursion_limit are snake_case
        result = convert(BASIC_SCHEMA, {
            "target": "openai-strict",
            "max_depth": 10,
            "recursion_limit": 5,
        })
        assert "schema" in result
