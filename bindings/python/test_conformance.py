"""Conformance fixture tests for the WASI-backed json-schema-llm SDK.

Loads fixtures from tests/conformance/fixtures.json and runs each fixture
through the appropriate engine method, asserting expected outcomes.
"""

import json
import os

import pytest

from json_schema_llm_wasi import SchemaLlmEngine, JslError, ConvertOptions

FIXTURES_PATH = os.path.join(
    os.path.dirname(__file__), "..", "..", "tests", "conformance", "fixtures.json"
)


@pytest.fixture(scope="module")
def engine():
    with SchemaLlmEngine() as eng:
        yield eng


@pytest.fixture(scope="module")
def fixtures():
    with open(FIXTURES_PATH) as f:
        return json.load(f)



def _get_fixture(fixtures, suite, fixture_id):
    for fx in fixtures["suites"][suite]["fixtures"]:
        if fx["id"] == fixture_id:
            return fx
    raise ValueError(f"fixture {fixture_id} not found in suite {suite}")


def _options_from_dict(raw_opts: dict | None) -> ConvertOptions | None:
    """Convert a raw fixture options dict to ConvertOptions."""
    if not raw_opts:
        return None
    return ConvertOptions(
        target=raw_opts.get("target"),
        mode=raw_opts.get("mode"),
        max_depth=raw_opts.get("max-depth") or raw_opts.get("max_depth"),
        recursion_limit=raw_opts.get("recursion-limit") or raw_opts.get("recursion_limit"),
        polymorphism=raw_opts.get("polymorphism"),
    )


# ---------------------------------------------------------------------------
# Convert suite
# ---------------------------------------------------------------------------


def _run_convert(engine, fixtures, fixture_id):
    fx = _get_fixture(fixtures, "convert", fixture_id)
    inp = fx["input"]
    expected = fx["expected"]

    # Error case: schema_raw → raw FFI
    if "schema_raw" in inp:
        assert expected.get("is_error") is True
        opts_json = json.dumps(inp.get("options", {}))
        with pytest.raises(JslError) as exc_info:
            engine._call_jsl("jsl_convert", inp["schema_raw"], opts_json)

        err = exc_info.value
        if "error_has_keys" in expected:
            for key in expected["error_has_keys"]:
                assert getattr(err, key, None) is not None, f"error missing '{key}'"
        if "error_code" in expected:
            assert err.code == expected["error_code"]
        return

    # Normal convert
    opts = _options_from_dict(inp.get("options"))
    result = engine.convert(inp["schema"], opts)
    _assert_convert_expected(result, expected)


def _assert_convert_expected(result, expected):
    if "has_keys" in expected:
        for key in expected["has_keys"]:
            # Map fixture key names to typed attribute names
            attr_map = {
                "apiVersion": "api_version",
                "schema": "schema",
                "codec": "codec",
                "providerCompatErrors": "provider_compat_errors",
            }
            attr = attr_map.get(key, key)
            assert hasattr(result, attr), f"result missing attribute '{attr}'"

    if "apiVersion" in expected:
        assert result.api_version == expected["apiVersion"]

    if "schema_has_properties" in expected:
        assert "properties" in result.schema

    if "codec_has_schema_uri" in expected:
        assert result.codec is not None


# Generate parametrized tests dynamically
def _generate_convert_tests():
    with open(FIXTURES_PATH) as f:
        data = json.load(f)
    return [fx["id"] for fx in data["suites"]["convert"]["fixtures"]]


@pytest.mark.parametrize("fixture_id", _generate_convert_tests())
def test_conformance_convert(engine, fixtures, fixture_id):
    _run_convert(engine, fixtures, fixture_id)


# ---------------------------------------------------------------------------
# Roundtrip suite
# ---------------------------------------------------------------------------


def _generate_roundtrip_tests():
    with open(FIXTURES_PATH) as f:
        data = json.load(f)
    return [fx["id"] for fx in data["suites"]["roundtrip"]["fixtures"]]


@pytest.mark.parametrize("fixture_id", _generate_roundtrip_tests())
def test_conformance_roundtrip(engine, fixtures, fixture_id):
    fx = _get_fixture(fixtures, "roundtrip", fixture_id)
    inp = fx["input"]
    expected = fx["expected"]

    opts = _options_from_dict(inp.get("options"))
    convert_result = engine.convert(inp["schema"], opts)
    rehydrate_result = engine.rehydrate(
        inp["data"], convert_result.codec, inp["schema"]
    )

    if "has_keys" in expected:
        attr_map = {
            "apiVersion": "api_version",
            "data": "data",
            "warnings": "warnings",
        }
        for key in expected["has_keys"]:
            attr = attr_map.get(key, key)
            assert hasattr(rehydrate_result, attr), f"result missing attribute '{attr}'"

    if "apiVersion" in expected:
        assert rehydrate_result.api_version == expected["apiVersion"]

    if "data" in expected:
        assert rehydrate_result.data == expected["data"]

    if "data_user_name" in expected:
        assert rehydrate_result.data["user"]["name"] == expected["data_user_name"]

    if "data_value" in expected:
        assert rehydrate_result.data["value"] == expected["data_value"]

    if "warnings_is_array" in expected:
        assert isinstance(rehydrate_result.warnings, list)


# ---------------------------------------------------------------------------
# Rehydrate error suite
# ---------------------------------------------------------------------------


def _generate_rehydrate_error_tests():
    with open(FIXTURES_PATH) as f:
        data = json.load(f)
    return [fx["id"] for fx in data["suites"]["rehydrate_error"]["fixtures"]]


@pytest.mark.parametrize("fixture_id", _generate_rehydrate_error_tests())
def test_conformance_rehydrate_error(engine, fixtures, fixture_id):
    fx = _get_fixture(fixtures, "rehydrate_error", fixture_id)
    inp = fx["input"]
    expected = fx["expected"]

    assert expected.get("is_error") is True

    data_json = json.dumps(inp["data"])
    schema_json = json.dumps(inp["schema"])
    codec_arg = inp.get("codec_raw", "{}")

    with pytest.raises(JslError) as exc_info:
        engine._call_jsl("jsl_rehydrate", data_json, codec_arg, schema_json)

    err = exc_info.value
    if "error_has_keys" in expected:
        for key in expected["error_has_keys"]:
            assert getattr(err, key, None) is not None, f"error missing '{key}'"


# ---------------------------------------------------------------------------
# List components suite
# ---------------------------------------------------------------------------


def _generate_list_components_tests():
    with open(FIXTURES_PATH) as f:
        data = json.load(f)
    return [fx["id"] for fx in data["suites"]["list_components"]["fixtures"]]


@pytest.mark.parametrize("fixture_id", _generate_list_components_tests())
def test_conformance_list_components(engine, fixtures, fixture_id):
    fx = _get_fixture(fixtures, "list_components", fixture_id)
    inp = fx["input"]
    expected = fx["expected"]

    # Error case: schema_raw
    if "schema_raw" in inp:
        assert expected.get("is_error") is True
        with pytest.raises(JslError) as exc_info:
            engine._call_jsl("jsl_list_components", inp["schema_raw"])
        err = exc_info.value
        if "error_has_keys" in expected:
            for key in expected["error_has_keys"]:
                assert getattr(err, key, None) is not None, f"error missing '{key}'"
        return

    result = engine.list_components(inp["schema"])

    if "apiVersion" in expected:
        assert result.api_version == expected["apiVersion"]

    if "components" in expected:
        assert result.components == expected["components"]


# ---------------------------------------------------------------------------
# Extract component suite
# ---------------------------------------------------------------------------


def _generate_extract_component_tests():
    with open(FIXTURES_PATH) as f:
        data = json.load(f)
    return [fx["id"] for fx in data["suites"]["extract_component"]["fixtures"]]


@pytest.mark.parametrize("fixture_id", _generate_extract_component_tests())
def test_conformance_extract_component(engine, fixtures, fixture_id):
    fx = _get_fixture(fixtures, "extract_component", fixture_id)
    inp = fx["input"]
    expected = fx["expected"]

    # Error case
    if expected.get("is_error"):
        if "schema_raw" in inp:
            with pytest.raises(JslError) as exc_info:
                engine._call_jsl(
                    "jsl_extract_component", inp["schema_raw"], inp["pointer"], "{}"
                )
        else:
            with pytest.raises(JslError) as exc_info:
                engine.extract_component(inp["schema"], inp["pointer"])

        err = exc_info.value
        if "error_has_keys" in expected:
            for key in expected["error_has_keys"]:
                assert getattr(err, key, None) is not None, f"error missing '{key}'"
        return

    result = engine.extract_component(
        inp["schema"], inp["pointer"], inp.get("options")
    )

    if "apiVersion" in expected:
        assert result.api_version == expected["apiVersion"]
    if "pointer" in expected:
        assert result.pointer == expected["pointer"]
    if "schema_is_object" in expected:
        assert isinstance(result.schema, dict)
    if "dependency_count" in expected:
        assert result.dependency_count == expected["dependency_count"]
    if "dependency_count_gte" in expected:
        assert result.dependency_count >= expected["dependency_count_gte"]


# ---------------------------------------------------------------------------
# Convert all components suite
# ---------------------------------------------------------------------------


def _generate_convert_all_tests():
    with open(FIXTURES_PATH) as f:
        data = json.load(f)
    return [fx["id"] for fx in data["suites"]["convert_all_components"]["fixtures"]]


@pytest.mark.parametrize("fixture_id", _generate_convert_all_tests())
def test_conformance_convert_all_components(engine, fixtures, fixture_id):
    fx = _get_fixture(fixtures, "convert_all_components", fixture_id)
    inp = fx["input"]
    expected = fx["expected"]

    # Error case: schema_raw
    if "schema_raw" in inp:
        assert expected.get("is_error") is True
        conv_opts = json.dumps(inp.get("convert_options", {}))
        ext_opts = json.dumps(inp.get("extract_options", {}))
        with pytest.raises(JslError) as exc_info:
            engine._call_jsl(
                "jsl_convert_all_components", inp["schema_raw"], conv_opts, ext_opts
            )
        err = exc_info.value
        if "error_has_keys" in expected:
            for key in expected["error_has_keys"]:
                assert getattr(err, key, None) is not None, f"error missing '{key}'"
        return

    conv_opts = _options_from_dict(inp.get("convert_options"))
    result = engine.convert_all_components(
        inp["schema"],
        conv_opts,
        inp.get("extract_options"),
    )

    if "apiVersion" in expected:
        assert result.api_version == expected["apiVersion"]
    if "full_is_object" in expected:
        assert isinstance(result.full, dict)
    if "components_count" in expected:
        assert len(result.components) == expected["components_count"]
