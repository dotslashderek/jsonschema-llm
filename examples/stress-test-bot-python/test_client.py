"""Tests for the Python stress test client (no API key required).

Validates CLI parsing, schema loading, and binding integration
without making any LLM API calls.
"""

import os
import subprocess
import sys


SCRIPT_PATH = os.path.join(os.path.dirname(__file__), "main.py")
SCHEMAS_DIR = os.path.join(
    os.path.dirname(__file__), "..", "..", "tests", "schemas", "stress"
)


class TestCliParsing:
    """Verify CLI argument handling matches TS client interface."""

    def test_help_flag_exits_zero(self):
        result = subprocess.run(
            [sys.executable, SCRIPT_PATH, "--help"],
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0
        assert "--count" in result.stdout
        assert "--seed" in result.stdout
        assert "--model" in result.stdout
        assert "--schemas-dir" in result.stdout

    def test_invalid_count_exits_nonzero(self):
        result = subprocess.run(
            [sys.executable, SCRIPT_PATH, "--count", "-1"],
            capture_output=True,
            text=True,
        )
        assert result.returncode != 0


class TestSchemaLoading:
    """Verify schema directory discovery and file loading."""

    def test_schema_dir_exists(self):
        assert os.path.isdir(SCHEMAS_DIR)

    def test_schema_dir_contains_json_files(self):
        files = [f for f in os.listdir(SCHEMAS_DIR) if f.endswith(".json")]
        assert len(files) > 0


class TestBindingIntegration:
    """Verify PyO3 binding is importable and functional."""

    def test_import_json_schema_llm(self):
        import json_schema_llm

        assert hasattr(json_schema_llm, "convert")
        assert hasattr(json_schema_llm, "rehydrate")

    def test_convert_round_trip(self):
        from json_schema_llm import convert, rehydrate

        schema = {"type": "object", "properties": {"name": {"type": "string"}}}
        result = convert(schema, {"target": "openai-strict"})
        assert "schema" in result
        assert "codec" in result

        rh = rehydrate({"name": "test"}, result["codec"], schema)
        assert rh["data"]["name"] == "test"
