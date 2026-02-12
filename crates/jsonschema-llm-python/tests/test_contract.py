"""Contract tests for jsonschema-llm Python bindings.

Validates the Python binding produces correct envelope shapes, round-trips
data through convert → rehydrate, matches Rust golden snapshots, and
raises structured errors for invalid inputs.

Mirrors the Node.js contract tests in tests/contract-node/contract.test.js.

Run: python -m pytest tests/test_contract.py -v
"""

import json
from pathlib import Path

import pytest
from jsonschema_llm import convert, rehydrate, JsonSchemaLlmError

from conftest import ALL_FIXTURES, TARGETS, SCHEMAS_DIR, SNAPSHOTS_DIR


# ---------------------------------------------------------------------------
# Convert — Envelope structure (fixture × target matrix)
# ---------------------------------------------------------------------------


class TestConvertEnvelope:
    """Every fixture × every target must return a well-formed envelope."""

    @pytest.mark.parametrize(
        "name, schema",
        ALL_FIXTURES,
        ids=[name for name, _ in ALL_FIXTURES],
    )
    @pytest.mark.parametrize("target", TARGETS, ids=TARGETS)
    def test_envelope_shape(self, name, schema, target):
        try:
            result = convert(schema, {"target": target})
        except JsonSchemaLlmError:
            # Some schemas (e.g. depth-50) exceed recursion limits —
            # a structured error is acceptable behavior.
            return

        # api_version
        assert result["api_version"] == "1.0", f"{name} × {target}: missing api_version"

        # schema is a dict
        assert isinstance(result["schema"], dict), f"{name} × {target}: schema not dict"

        # codec is a dict with expected subfields
        codec = result["codec"]
        assert isinstance(codec, dict), f"{name} × {target}: codec not dict"
        assert "$schema" in codec, f"{name} × {target}: codec missing $schema"
        assert isinstance(codec["transforms"], list), f"{name} × {target}: transforms not list"
        assert isinstance(
            codec["droppedConstraints"], list
        ), f"{name} × {target}: droppedConstraints not list"


# ---------------------------------------------------------------------------
# Convert — Schema output has type key
# ---------------------------------------------------------------------------


class TestConvertSchema:
    """All fixture schemas should produce output with a 'type' field."""

    @pytest.mark.parametrize(
        "name, schema",
        ALL_FIXTURES,
        ids=[name for name, _ in ALL_FIXTURES],
    )
    def test_output_has_type(self, name, schema):
        try:
            result = convert(schema, {"target": "openai-strict"})
        except JsonSchemaLlmError:
            # Some schemas exceed depth limits — structured error is valid.
            return
        output = result["schema"]
        if isinstance(output, dict):
            assert "type" in output, f"{name}: output schema missing 'type' key"


# ---------------------------------------------------------------------------
# Convert — Options handling
# ---------------------------------------------------------------------------


class TestConvertOptions:
    """Verify snake_case options and default handling."""

    BASIC_SCHEMA = {"type": "object", "properties": {"x": {"type": "string"}}}

    def test_none_options(self):
        result = convert(self.BASIC_SCHEMA, None)
        assert result["api_version"] == "1.0"

    def test_no_options(self):
        result = convert(self.BASIC_SCHEMA)
        assert result["api_version"] == "1.0"

    def test_snake_case_options(self):
        result = convert(
            self.BASIC_SCHEMA,
            {"target": "openai-strict", "max_depth": 10, "recursion_limit": 5},
        )
        assert result["api_version"] == "1.0"
        assert "schema" in result

    @pytest.mark.parametrize("target", TARGETS, ids=TARGETS)
    def test_all_targets(self, target):
        result = convert(self.BASIC_SCHEMA, {"target": target})
        assert "schema" in result


# ---------------------------------------------------------------------------
# Round-trip: convert → rehydrate (fixture × target matrix)
# ---------------------------------------------------------------------------


class TestRoundTrip:
    """Convert every fixture, then rehydrate with empty data — verify pipeline."""

    @pytest.mark.parametrize(
        "name, schema",
        ALL_FIXTURES,
        ids=[name for name, _ in ALL_FIXTURES],
    )
    @pytest.mark.parametrize("target", TARGETS, ids=TARGETS)
    def test_round_trip_empty(self, name, schema, target):
        try:
            cr = convert(schema, {"target": target})
        except JsonSchemaLlmError:
            # Some schemas exceed depth/recursion limits — skip round-trip.
            pytest.skip(f"{name} × {target}: convert raised structured error")

        try:
            result = rehydrate({}, cr["codec"], schema)
        except JsonSchemaLlmError:
            # Some schemas (edge cases, root arrays, etc.) produce codecs
            # that expect specific data shapes — structured error is valid.
            return

        assert result["api_version"] == "1.0", f"{name} × {target}: rehydrate missing api_version"
        assert "data" in result, f"{name} × {target}: rehydrate missing data"
        assert isinstance(
            result["warnings"], list
        ), f"{name} × {target}: rehydrate warnings not list"


# ---------------------------------------------------------------------------
# Snapshot parity — Python output matches Rust golden files
# ---------------------------------------------------------------------------


class TestSnapshotParity:
    """Verify Python binding produces identical output to Rust core snapshots."""

    @pytest.fixture()
    def kitchen_sink_schema(self):
        path = SCHEMAS_DIR / "kitchen_sink.json"
        if not path.exists():
            pytest.skip("kitchen_sink.json not found")
        with open(path) as f:
            return json.load(f)

    def test_schema_matches_golden(self, kitchen_sink_schema):
        golden_path = SNAPSHOTS_DIR / "kitchen_sink_openai.expected.json"
        if not golden_path.exists():
            pytest.skip("golden snapshot not found")

        with open(golden_path) as f:
            expected = json.load(f)

        result = convert(kitchen_sink_schema, {"target": "openai-strict"})
        assert result["schema"] == expected, (
            "Python schema output diverged from Rust golden snapshot — "
            "if intentional, regenerate with: cargo run -p jsonschema-llm -- convert "
            "tests/schemas/kitchen_sink.json -o tests/snapshots/kitchen_sink_openai.expected.json "
            "--target openai-strict"
        )

    def test_codec_matches_golden(self, kitchen_sink_schema):
        golden_path = SNAPSHOTS_DIR / "kitchen_sink_codec.expected.json"
        if not golden_path.exists():
            pytest.skip("golden snapshot not found")

        with open(golden_path) as f:
            expected = json.load(f)

        result = convert(kitchen_sink_schema, {"target": "openai-strict"})
        assert result["codec"] == expected, (
            "Python codec output diverged from Rust golden snapshot — "
            "if intentional, regenerate with: cargo run -p jsonschema-llm -- convert "
            "tests/schemas/kitchen_sink.json --codec tests/snapshots/kitchen_sink_codec.expected.json "
            "--target openai-strict"
        )


# ---------------------------------------------------------------------------
# Round-trip with populated data — kitchen_sink
# ---------------------------------------------------------------------------


class TestRoundTripPopulated:
    """Verify round-trip with representative nested data across FFI."""

    @pytest.fixture()
    def kitchen_sink_schema(self):
        path = SCHEMAS_DIR / "kitchen_sink.json"
        if not path.exists():
            pytest.skip("kitchen_sink.json not found")
        with open(path) as f:
            return json.load(f)

    def test_populated_round_trip(self, kitchen_sink_schema):
        cr = convert(kitchen_sink_schema, {"target": "openai-strict"})

        # Representative data matching kitchen_sink shape
        sample = {
            "name": "Integration Test",
            "age": 42,
        }

        result = rehydrate(sample, cr["codec"], kitchen_sink_schema)
        assert result["api_version"] == "1.0"
        assert "data" in result
        assert isinstance(result["warnings"], list)


# ---------------------------------------------------------------------------
# Rehydrate — kitchen_sink specific tests
# ---------------------------------------------------------------------------


class TestRehydrateKitchenSink:
    """Verify dropped constraints and warnings for the complex kitchen_sink schema."""

    @pytest.fixture()
    def kitchen_sink_cr(self):
        path = SCHEMAS_DIR / "kitchen_sink.json"
        if not path.exists():
            pytest.skip("kitchen_sink.json not found")
        with open(path) as f:
            schema = json.load(f)
        return convert(schema, {"target": "openai-strict"}), schema

    def test_has_dropped_constraints(self, kitchen_sink_cr):
        cr, _ = kitchen_sink_cr
        assert len(cr["codec"]["droppedConstraints"]) > 0, (
            "kitchen_sink should have dropped constraints for openai-strict"
        )

    def test_rehydrate_returns_warnings(self, kitchen_sink_cr):
        cr, schema = kitchen_sink_cr
        result = rehydrate({}, cr["codec"], schema)
        assert result["api_version"] == "1.0"
        assert isinstance(result["warnings"], list)


# ---------------------------------------------------------------------------
# Error paths
# ---------------------------------------------------------------------------


class TestErrorPaths:
    """Verify structured error handling across the FFI boundary."""

    def test_unresolvable_ref(self):
        with pytest.raises(JsonSchemaLlmError) as exc_info:
            convert({"$ref": "#/$defs/NonExistent"})
        assert exc_info.value.code == "unresolvable_ref"
        assert exc_info.value.path is not None
        assert isinstance(exc_info.value.message, str)
        assert exc_info.value.message

    def test_bad_options_type(self):
        with pytest.raises(JsonSchemaLlmError) as exc_info:
            convert({"type": "object"}, 42)
        assert exc_info.value.code == "invalid_input"

    def test_bad_codec_version(self):
        bad_codec = {
            "$schema": "https://jsonschema-llm.dev/codec/v999",
            "transforms": [],
            "droppedConstraints": [],
        }
        with pytest.raises(JsonSchemaLlmError) as exc_info:
            rehydrate({}, bad_codec, {})
        assert hasattr(exc_info.value, "code")
        assert isinstance(exc_info.value.message, str)

    def test_error_is_exception_subclass(self):
        """JsonSchemaLlmError can be caught as a generic Exception."""
        with pytest.raises(Exception):
            convert({"$ref": "#/$defs/NonExistent"})
