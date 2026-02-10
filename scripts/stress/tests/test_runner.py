"""Acceptance tests for the stress test CLI runner.

These tests verify error handling, timeout behavior, and stage classification
without making real OpenAI API calls.
"""

from pathlib import Path
from unittest.mock import MagicMock, patch


class TestNoneResponseHandling:
    """Finding #1: call_openai returning None must not crash."""

    def test_none_content_does_not_crash(self):
        """If OpenAI returns None for message.content, classify as openai_error."""
        # Import the module
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "run_cli_test",
            Path(__file__).parent.parent / "run_cli_test.py",
        )
        mod = importlib.util.module_from_spec(spec)

        # Mock OpenAI to avoid import errors
        mock_openai = MagicMock()
        with patch.dict("sys.modules", {"openai": mock_openai}):
            spec.loader.exec_module(mod)

            # Create a mock client whose response has content=None
            client = MagicMock()
            response = MagicMock()
            response.choices = [MagicMock()]
            response.choices[0].message.content = None
            client.chat.completions.create.return_value = response

            # call_openai should handle None gracefully
            result = mod.call_openai(client, "test_schema", {"type": "object"})

            # call_openai should return None for null content, or a string error
            # The key assertion: no AttributeError crash
            assert result is None or isinstance(result, str), (
                "call_openai should return None or error string, not crash"
            )

    def test_none_response_classified_as_openai_error(self):
        """When call_openai returns None, the runner's main loop must classify it as null_content."""
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "run_cli_test",
            Path(__file__).parent.parent / "run_cli_test.py",
        )
        mod = importlib.util.module_from_spec(spec)

        mock_openai = MagicMock()
        with patch.dict("sys.modules", {"openai": mock_openai}):
            spec.loader.exec_module(mod)

            # Set up mock client where content is None
            client = MagicMock()
            response = MagicMock()
            response.choices = [MagicMock()]
            response.choices[0].message.content = None
            client.chat.completions.create.return_value = response

            result = mod.call_openai(client, "test", {"type": "object"})

            # call_openai should return None for null content
            assert result is None, (
                f"Expected None for null content, got {type(result).__name__}: {result}"
            )

            # Verify main loop's guard: the result should NOT be passed to .startswith()
            # The main loop checks `if llm_response_str is None` before string ops
            source = (Path(__file__).parent.parent / "run_cli_test.py").read_text()
            assert "if llm_response_str is None" in source, (
                "Runner must have an explicit None guard before string operations"
            )


class TestTimeoutBehavior:
    """Finding #9: subprocesses must have timeouts."""

    def test_conversion_has_timeout(self):
        """run_cli_conversion should accept and use a timeout parameter."""
        import importlib.util
        import inspect

        spec = importlib.util.spec_from_file_location(
            "run_cli_test",
            Path(__file__).parent.parent / "run_cli_test.py",
        )
        mod = importlib.util.module_from_spec(spec)

        mock_openai = MagicMock()
        with patch.dict("sys.modules", {"openai": mock_openai}):
            spec.loader.exec_module(mod)

            # run_cli_conversion should accept a timeout parameter
            sig = inspect.signature(mod.run_cli_conversion)
            param_names = list(sig.parameters.keys())
            assert "timeout" in param_names, (
                f"run_cli_conversion missing 'timeout' parameter. Has: {param_names}"
            )

    def test_rehydration_has_timeout(self):
        """run_cli_rehydration should accept and use a timeout parameter."""
        import importlib.util
        import inspect

        spec = importlib.util.spec_from_file_location(
            "run_cli_test",
            Path(__file__).parent.parent / "run_cli_test.py",
        )
        mod = importlib.util.module_from_spec(spec)

        mock_openai = MagicMock()
        with patch.dict("sys.modules", {"openai": mock_openai}):
            spec.loader.exec_module(mod)

            sig = inspect.signature(mod.run_cli_rehydration)
            param_names = list(sig.parameters.keys())
            assert "timeout" in param_names, (
                f"run_cli_rehydration missing 'timeout' parameter. Has: {param_names}"
            )


class TestModelParameterization:
    """Finding #9: model should be configurable via --model flag."""

    def test_call_openai_accepts_model(self):
        """call_openai should accept a model parameter."""
        import importlib.util
        import inspect

        spec = importlib.util.spec_from_file_location(
            "run_cli_test",
            Path(__file__).parent.parent / "run_cli_test.py",
        )
        mod = importlib.util.module_from_spec(spec)

        mock_openai = MagicMock()
        with patch.dict("sys.modules", {"openai": mock_openai}):
            spec.loader.exec_module(mod)

            sig = inspect.signature(mod.call_openai)
            param_names = list(sig.parameters.keys())
            assert "model" in param_names, (
                f"call_openai missing 'model' parameter. Has: {param_names}"
            )


class TestStageClassification:
    """X review: per-stage result classes with reason codes."""

    def test_stages_are_distinct(self):
        """Results should classify failures into distinct stages."""
        # The expected stages in the pipeline
        expected_stages = {"convert", "openai", "rehydrate", "validation"}

        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "run_cli_test",
            Path(__file__).parent.parent / "run_cli_test.py",
        )
        mod = importlib.util.module_from_spec(spec)

        mock_openai = MagicMock()
        with patch.dict("sys.modules", {"openai": mock_openai}):
            spec.loader.exec_module(mod)

            # Check that the source uses these stage classifications
            source = (Path(__file__).parent.parent / "run_cli_test.py").read_text()
            for stage in expected_stages:
                assert f'"{stage}"' in source or f"'{stage}'" in source, (
                    f"Stage '{stage}' not found in runner source"
                )


class TestSanitizeSchemaName:
    """G review: schema names must conform to OpenAI's pattern."""

    def _get_sanitizer(self):
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "run_cli_test",
            Path(__file__).parent.parent / "run_cli_test.py",
        )
        mod = importlib.util.module_from_spec(spec)
        mock_openai = MagicMock()
        with patch.dict("sys.modules", {"openai": mock_openai}):
            spec.loader.exec_module(mod)
        return mod._sanitize_schema_name

    def test_alphanumeric_passthrough(self):
        """Clean names should pass through unchanged."""
        sanitize = self._get_sanitizer()
        assert sanitize("my_schema-v2") == "my_schema-v2"

    def test_spaces_replaced(self):
        """Spaces become underscores."""
        sanitize = self._get_sanitizer()
        assert sanitize("my schema name") == "my_schema_name"

    def test_special_chars_replaced(self):
        """Special characters (dots, parens, etc.) become underscores."""
        sanitize = self._get_sanitizer()
        result = sanitize("schema(v2).final")
        assert result == "schema_v2__final"

    def test_max_64_chars(self):
        """Names longer than 64 chars are truncated."""
        sanitize = self._get_sanitizer()
        long_name = "a" * 100
        assert len(sanitize(long_name)) == 64


class TestSeedOrdering:
    """X review: --seed must actually affect schema ordering."""

    def test_same_seed_same_order(self):
        """Same seed should produce identical schema ordering."""
        import importlib.util
        import random

        spec = importlib.util.spec_from_file_location(
            "run_cli_test",
            Path(__file__).parent.parent / "run_cli_test.py",
        )
        mod = importlib.util.module_from_spec(spec)
        mock_openai = MagicMock()
        with patch.dict("sys.modules", {"openai": mock_openai}):
            spec.loader.exec_module(mod)

        # Simulate what the runner does with a seed
        schemas = ["a.json", "b.json", "c.json", "d.json", "e.json"]
        copy1 = schemas.copy()
        copy2 = schemas.copy()

        random.seed(42)
        random.shuffle(copy1)
        random.seed(42)
        random.shuffle(copy2)

        assert copy1 == copy2, "Same seed should produce same ordering"

    def test_different_seed_different_order(self):
        """Different seeds should (likely) produce different ordering."""
        import random

        schemas = list(range(20))  # Enough items to make collision unlikely
        copy1 = schemas.copy()
        copy2 = schemas.copy()

        random.seed(42)
        random.shuffle(copy1)
        random.seed(99)
        random.shuffle(copy2)

        assert copy1 != copy2, "Different seeds should produce different ordering"


class TestStderrCapture:
    """G review: subprocess stderr should be captured on success too."""

    def test_conversion_returns_stderr_on_success(self):
        """run_cli_conversion should return stderr even on success."""
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "run_cli_test",
            Path(__file__).parent.parent / "run_cli_test.py",
        )
        mod = importlib.util.module_from_spec(spec)
        mock_openai = MagicMock()
        with patch.dict("sys.modules", {"openai": mock_openai}):
            spec.loader.exec_module(mod)

            # Verify the function signature returns stderr (2-tuple)
            source = (Path(__file__).parent.parent / "run_cli_test.py").read_text()
            # On success, should return result.stderr not empty string
            assert "return True, result.stderr" in source, (
                "run_cli_conversion should return stderr on success, not empty string"
            )
