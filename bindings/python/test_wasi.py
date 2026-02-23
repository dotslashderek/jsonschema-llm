"""Tests for the WASI-backed json-schema-llm SDK."""

import json
import pytest

from json_schema_llm_wasi import (
    SchemaLlmEngine,
    JslError,
    ConvertResult,
    ConvertOptions,
    RehydrateResult,
)


@pytest.fixture
def engine():
    with SchemaLlmEngine() as eng:
        yield eng


def test_convert_returns_typed_result(engine):
    """Basic schema conversion returns a ConvertResult."""
    schema = {
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer", "minimum": 0},
        },
        "required": ["name", "age"],
    }
    result = engine.convert(schema)
    assert isinstance(result, ConvertResult)
    assert result.api_version
    assert isinstance(result.schema, dict)
    assert isinstance(result.codec, dict)
    assert result.schema.get("additionalProperties") is not None


def test_convert_with_options(engine):
    """ConvertOptions are passed through to the WASM binary."""
    schema = {"type": "object", "properties": {"x": {"type": "string"}}}
    opts = ConvertOptions(target="openai-strict", max_depth=50)
    result = engine.convert(schema, opts)
    assert isinstance(result, ConvertResult)
    assert result.schema is not None


def test_convert_with_builder_options(engine):
    """ConvertOptions.builder() fluent API works end-to-end."""
    schema = {"type": "object", "properties": {"x": {"type": "string"}}}
    opts = ConvertOptions.builder().target("openai-strict").build()
    result = engine.convert(schema, opts)
    assert isinstance(result, ConvertResult)


def test_convert_error(engine):
    """Invalid JSON schema input returns structured error."""
    with pytest.raises(JslError) as exc_info:
        engine._call_jsl("jsl_convert", "NOT VALID JSON", "{}")
    assert exc_info.value.code != ""
    assert exc_info.value.message != ""


def test_roundtrip(engine):
    """Convert + rehydrate roundtrip preserves data with typed results."""
    schema = {
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer", "minimum": 0},
        },
        "required": ["name", "age"],
    }
    convert_result = engine.convert(schema)

    data = {"name": "Ada", "age": 36}
    rehydrate_result = engine.rehydrate(data, convert_result.codec, schema)

    assert isinstance(rehydrate_result, RehydrateResult)
    assert rehydrate_result.api_version
    assert rehydrate_result.data["name"] == "Ada"


def test_rehydrate_error(engine):
    """Rehydrate with invalid codec returns error."""
    schema = {"type": "object"}
    data = {"key": "value"}
    with pytest.raises(JslError):
        engine._call_jsl("jsl_rehydrate", json.dumps(data), "NOT VALID JSON", json.dumps(schema))


def test_multiple_calls(engine):
    """Engine handles sequential calls with typed results."""
    schema = {"type": "object", "properties": {"x": {"type": "number"}}}
    for _ in range(5):
        result = engine.convert(schema)
        assert isinstance(result, ConvertResult)
        assert result.schema is not None


def test_real_world_schema(engine):
    """Complex nested schema converts and roundtrips with typed results."""
    schema = {
        "type": "object",
        "properties": {
            "user": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "emails": {"type": "array", "items": {"type": "string"}},
                },
                "required": ["name"],
            },
            "role": {"type": "string", "enum": ["admin", "user", "guest"]},
        },
        "required": ["user", "role"],
    }
    result = engine.convert(schema)
    assert isinstance(result, ConvertResult)
    assert result.api_version

    data = {
        "user": {"name": "Ada Lovelace", "emails": ["ada@example.com"]},
        "role": "admin",
    }
    rehydrated = engine.rehydrate(data, result.codec, schema)
    assert isinstance(rehydrated, RehydrateResult)
    assert rehydrated.data["user"]["name"] == "Ada Lovelace"


def test_import_smoke():
    """All public API types are importable from the package."""
    from json_schema_llm_wasi import (
        SchemaLlmEngine,
        JslError,
        ConvertResult,
        RehydrateResult,
        ConvertOptions,
        ListComponentsResult,
        ExtractComponentResult,
        ConvertAllComponentsResult,
    )
    # Just verify they're all real classes
    assert SchemaLlmEngine is not None
    assert JslError is not None
    assert ConvertResult is not None
    assert RehydrateResult is not None
    assert ConvertOptions is not None
    assert ListComponentsResult is not None
    assert ExtractComponentResult is not None
    assert ConvertAllComponentsResult is not None
