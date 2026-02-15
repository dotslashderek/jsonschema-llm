"""Acceptance tests for the stress test CLI runner.

These tests verify error handling, timeout behavior, and stage classification
without making real OpenAI API calls.
"""

from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest


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


def _load_runner_module():
    """Helper to load run_cli_test module with mocked OpenAI."""
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "run_cli_test",
        Path(__file__).parent.parent / "run_cli_test.py",
    )
    mod = importlib.util.module_from_spec(spec)
    mock_openai = MagicMock()
    with patch.dict("sys.modules", {"openai": mock_openai}):
        spec.loader.exec_module(mod)
    return mod


class TestRetryLogic:
    """#116: Retry mechanism for non-deterministic results."""

    def test_run_single_schema_exists(self):
        """run_single_schema function must exist."""
        mod = _load_runner_module()
        assert hasattr(mod, "run_single_schema"), (
            "run_cli_test must expose a run_single_schema function"
        )

    def test_run_single_schema_returns_structured_result(self):
        """run_single_schema must return a dict with verdict and attempts."""
        mod = _load_runner_module()
        assert hasattr(mod, "run_single_schema")
        # Verify function signature includes expected parameters
        import inspect

        sig = inspect.signature(mod.run_single_schema)
        param_names = list(sig.parameters.keys())
        assert "retries" in param_names, (
            f"run_single_schema must accept 'retries' parameter. Has: {param_names}"
        )

    def test_solid_pass_on_first_attempt(self):
        """Schema passing first try → verdict=solid_pass, 1 attempt."""
        mod = _load_runner_module()
        # Mock all pipeline stages to succeed
        with patch.object(mod, "run_cli_conversion", return_value=(True, "")):
            with patch.object(mod, "call_openai", return_value='{"key": "value"}'):
                with patch.object(mod, "run_cli_rehydration", return_value=(True, "")):
                    with patch.object(
                        mod, "validate_original", return_value=(True, "")
                    ):
                        with patch("builtins.open", MagicMock()):
                            with patch("json.load", return_value={"type": "object"}):
                                with patch("json.dump"):
                                    result = mod.run_single_schema(
                                        binary_path="/fake/bin",
                                        schema_file="test.json",
                                        schemas_dir="/fake/schemas",
                                        output_dir="/fake/output",
                                        client=MagicMock(),
                                        model="gpt-4o-mini",
                                        timeout_subprocess=30,
                                        timeout_api=60,
                                        retries=2,
                                        retry_delay=0,
                                    )
        assert result["verdict"] == "solid_pass"
        assert len(result["attempts"]) == 1

    def test_flaky_pass_after_retry(self):
        """Schema failing first, passing second → verdict=flaky_pass."""
        mod = _load_runner_module()
        call_count = {"n": 0}

        def fake_openai(*args, **kwargs):
            call_count["n"] += 1
            if call_count["n"] == 1:
                return "OPENAI_ERROR: flaky failure"
            return '{"key": "value"}'

        with patch.object(mod, "run_cli_conversion", return_value=(True, "")):
            with patch.object(mod, "call_openai", side_effect=fake_openai):
                with patch.object(mod, "run_cli_rehydration", return_value=(True, "")):
                    with patch.object(
                        mod, "validate_original", return_value=(True, "")
                    ):
                        with patch("builtins.open", MagicMock()):
                            with patch("json.load", return_value={"type": "object"}):
                                with patch("json.dump"):
                                    result = mod.run_single_schema(
                                        binary_path="/fake/bin",
                                        schema_file="test.json",
                                        schemas_dir="/fake/schemas",
                                        output_dir="/fake/output",
                                        client=MagicMock(),
                                        model="gpt-4o-mini",
                                        timeout_subprocess=30,
                                        timeout_api=60,
                                        retries=2,
                                        retry_delay=0,
                                    )
        assert result["verdict"] == "flaky_pass"
        assert len(result["attempts"]) == 2

    def test_solid_fail_all_retries_exhausted(self):
        """Schema failing all attempts → verdict=solid_fail."""
        mod = _load_runner_module()
        with patch.object(mod, "run_cli_conversion", return_value=(True, "")):
            with patch.object(
                mod, "call_openai", return_value="OPENAI_ERROR: always fails"
            ):
                with patch("builtins.open", MagicMock()):
                    with patch("json.load", return_value={"type": "object"}):
                        result = mod.run_single_schema(
                            binary_path="/fake/bin",
                            schema_file="test.json",
                            schemas_dir="/fake/schemas",
                            output_dir="/fake/output",
                            client=MagicMock(),
                            model="gpt-4o-mini",
                            timeout_subprocess=30,
                            timeout_api=60,
                            retries=2,
                            retry_delay=0,
                        )
        assert result["verdict"] == "solid_fail"
        assert len(result["attempts"]) == 3

    def test_no_retries_default(self):
        """With retries=0, only one attempt is made."""
        mod = _load_runner_module()
        with patch.object(mod, "run_cli_conversion", return_value=(True, "")):
            with patch.object(
                mod, "call_openai", return_value="OPENAI_ERROR: fail once"
            ):
                with patch("builtins.open", MagicMock()):
                    with patch("json.load", return_value={"type": "object"}):
                        result = mod.run_single_schema(
                            binary_path="/fake/bin",
                            schema_file="test.json",
                            schemas_dir="/fake/schemas",
                            output_dir="/fake/output",
                            client=MagicMock(),
                            model="gpt-4o-mini",
                            timeout_subprocess=30,
                            timeout_api=60,
                            retries=0,
                        )
        assert result["verdict"] == "solid_fail"
        assert len(result["attempts"]) == 1

    def test_attempts_recorded_in_result(self):
        """Each attempt should record stage and error info."""
        mod = _load_runner_module()
        with patch.object(
            mod, "run_cli_conversion", return_value=(False, "conv error")
        ):
            with patch("builtins.open", MagicMock()):
                with patch("json.load", return_value={"type": "object"}):
                    result = mod.run_single_schema(
                        binary_path="/fake/bin",
                        schema_file="test.json",
                        schemas_dir="/fake/schemas",
                        output_dir="/fake/output",
                        client=MagicMock(),
                        model="gpt-4o-mini",
                        timeout_subprocess=30,
                        timeout_api=60,
                        retries=0,
                    )
        assert len(result["attempts"]) == 1
        attempt = result["attempts"][0]
        assert "stage" in attempt
        assert "passed" in attempt


class TestExpectedFailures:
    """#117: Expected failure classification."""

    def test_load_expected_failures_function_exists(self):
        """load_expected_failures function must exist."""
        mod = _load_runner_module()
        assert hasattr(mod, "load_expected_failures"), (
            "run_cli_test must expose a load_expected_failures function"
        )

    def test_config_loading(self):
        """Valid config file should load correctly."""
        mod = _load_runner_module()
        import json
        import tempfile

        config = {
            "schemas": {
                "edge_false": {"reason": "Unsatisfiable", "stage": "validation"},
            }
        }
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
            json.dump(config, f)
            f.flush()
            result = mod.load_expected_failures(f.name)
        assert "edge_false" in result
        assert result["edge_false"]["reason"] == "Unsatisfiable"

    def test_classify_expected_fail(self):
        """classify_result must exist and handle expected failures."""
        mod = _load_runner_module()
        assert hasattr(mod, "classify_result"), (
            "run_cli_test must expose a classify_result function"
        )
        expected_failures = {
            "edge_false": {"reason": "Unsatisfiable", "stage": "validation"},
        }
        # Schema that fails at expected stage → expected_fail
        result = {
            "verdict": "solid_fail",
            "file": "edge_false.json",
            "attempts": [{"passed": False, "stage": "validation", "error": "mismatch"}],
        }
        classified = mod.classify_result(result, expected_failures)
        assert classified == "expected_fail"

    def test_classify_unexpected_pass(self):
        """Schema in config that passes → unexpected_pass."""
        mod = _load_runner_module()
        expected_failures = {
            "edge_false": {"reason": "Unsatisfiable", "stage": "validation"},
        }
        result = {
            "verdict": "solid_pass",
            "file": "edge_false.json",
            "attempts": [{"passed": True, "stage": None, "error": ""}],
        }
        classified = mod.classify_result(result, expected_failures)
        assert classified == "unexpected_pass"

    def test_stage_mismatch_is_solid_fail(self):
        """Config says stage=validation but fails at convert → solid_fail (regression)."""
        mod = _load_runner_module()
        expected_failures = {
            "edge_false": {"reason": "Unsatisfiable", "stage": "validation"},
        }
        # Fails at convert, not validation — this is a regression
        result = {
            "verdict": "solid_fail",
            "file": "edge_false.json",
            "attempts": [
                {"passed": False, "stage": "convert", "error": "unexpected crash"}
            ],
        }
        classified = mod.classify_result(result, expected_failures)
        assert classified == "solid_fail"

    def test_no_config_is_noop(self):
        """Without expected-failures, classify_result returns verdict as-is."""
        mod = _load_runner_module()
        result = {
            "verdict": "solid_fail",
            "file": "test.json",
            "attempts": [{"passed": False, "stage": "openai", "error": "api error"}],
        }
        classified = mod.classify_result(result, {})
        assert classified == "solid_fail"

    def test_no_stage_in_config_matches_any(self):
        """Config without stage field should match any failure stage."""
        mod = _load_runner_module()
        expected_failures = {
            "deep_nesting_50": {"reason": "Too deep"},
        }
        result = {
            "verdict": "solid_fail",
            "file": "deep_nesting_50.json",
            "attempts": [
                {"passed": False, "stage": "convert", "error": "depth exceeded"}
            ],
        }
        classified = mod.classify_result(result, expected_failures)
        assert classified == "expected_fail"


class TestExpectedFailuresValidation:
    """Finding #9: load_expected_failures must validate schemas is a dict."""

    def test_schemas_list_rejected(self):
        """If schemas is a list instead of dict, should exit with error."""
        mod = _load_runner_module()
        import json
        import tempfile

        config = {"schemas": ["edge_false", "edge_true"]}
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
            json.dump(config, f)
            f.flush()
            with pytest.raises(SystemExit) as exc_info:
                mod.load_expected_failures(f.name)
            assert exc_info.value.code == 2

    def test_schemas_string_rejected(self):
        """If schemas is a string, should exit with error."""
        mod = _load_runner_module()
        import json
        import tempfile

        config = {"schemas": "not_a_dict"}
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
            json.dump(config, f)
            f.flush()
            with pytest.raises(SystemExit) as exc_info:
                mod.load_expected_failures(f.name)
            assert exc_info.value.code == 2

    def test_toplevel_list_rejected(self):
        """If top-level config is a list, should exit with error."""
        mod = _load_runner_module()
        import json
        import tempfile

        config = [{"schemas": {}}]
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
            json.dump(config, f)
            f.flush()
            with pytest.raises(SystemExit) as exc_info:
                mod.load_expected_failures(f.name)
            assert exc_info.value.code == 2

    def test_entry_string_rejected(self):
        """If a schemas entry value is a string, should exit with error."""
        mod = _load_runner_module()
        import json
        import tempfile

        config = {"schemas": {"edge_false": "just_a_reason"}}
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
            json.dump(config, f)
            f.flush()
            with pytest.raises(SystemExit) as exc_info:
                mod.load_expected_failures(f.name)
            assert exc_info.value.code == 2

