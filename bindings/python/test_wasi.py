"""Tests for the WASI-backed jsonschema-llm wrapper."""

import json
import pytest

from jsonschema_llm_wasi import Engine, JslError


@pytest.fixture
def engine():
    with Engine() as eng:
        yield eng


def test_convert_simple(engine):
    """Basic schema conversion succeeds."""
    schema = {
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer", "minimum": 0},
        },
        "required": ["name", "age"],
    }
    result = engine.convert(schema)
    assert "apiVersion" in result
    assert "schema" in result
    assert "codec" in result
    assert result["schema"].get("additionalProperties") is not None


def test_convert_error(engine):
    """Invalid JSON schema input returns structured error."""
    with pytest.raises(JslError) as exc_info:
        # Pass raw invalid JSON string — bypasses Python marshalling
        engine._call_jsl("jsl_convert", "NOT VALID JSON", "{}")
    assert exc_info.value.code != ""
    assert exc_info.value.message != ""


def test_roundtrip(engine):
    """Convert + rehydrate roundtrip preserves data."""
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
    rehydrate_result = engine.rehydrate(data, convert_result["codec"], schema)

    assert "apiVersion" in rehydrate_result
    assert "data" in rehydrate_result
    assert rehydrate_result["data"]["name"] == "Ada"


def test_rehydrate_error(engine):
    """Rehydrate with invalid codec returns error."""
    schema = {"type": "object"}
    data = {"key": "value"}
    with pytest.raises(JslError):
        engine._call_jsl("jsl_rehydrate", json.dumps(data), "NOT VALID JSON", json.dumps(schema))


def test_multiple_calls(engine):
    """Engine handles sequential calls."""
    schema = {"type": "object", "properties": {"x": {"type": "number"}}}
    for _ in range(5):
        result = engine.convert(schema)
        assert result["schema"] is not None


def test_real_world_schema(engine):
    """Complex nested schema converts and roundtrips."""
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
    assert result["apiVersion"]

    data = {
        "user": {"name": "Ada Lovelace", "emails": ["ada@example.com"]},
        "role": "admin",
    }
    rehydrated = engine.rehydrate(data, result["codec"], schema)
    assert rehydrated["data"]["user"]["name"] == "Ada Lovelace"
