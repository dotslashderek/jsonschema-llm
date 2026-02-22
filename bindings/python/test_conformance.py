"""Conformance fixture tests for the WASI-backed jsonschema-llm wrapper.

Loads fixtures from tests/conformance/fixtures.json and runs each fixture
through the appropriate engine method, asserting expected outcomes.
"""

import json
import os

import pytest

from json_schema_llm_wasi import Engine, JslError

FIXTURES_PATH = os.path.join(
    os.path.dirname(__file__), "..", "..", "tests", "conformance", "fixtures.json"
)


@pytest.fixture(scope="module")
def engine():
    with Engine() as eng:
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
    result = engine.convert(inp["schema"], inp.get("options") or {})
    _assert_convert_expected(result, expected)


def _assert_convert_expected(result, expected):
    if "has_keys" in expected:
        for key in expected["has_keys"]:
            assert key in result, f"result missing key '{key}'"

    if "apiVersion" in expected:
        assert result["apiVersion"] == expected["apiVersion"]

    if "schema_has_properties" in expected:
        assert "properties" in result["schema"]

    if "codec_has_schema_uri" in expected:
        assert result["codec"] is not None


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

    convert_result = engine.convert(inp["schema"], inp.get("options") or {})
    rehydrate_result = engine.rehydrate(
        inp["data"], convert_result["codec"], inp["schema"]
    )

    if "has_keys" in expected:
        for key in expected["has_keys"]:
            assert key in rehydrate_result, f"result missing key '{key}'"

    if "apiVersion" in expected:
        assert rehydrate_result["apiVersion"] == expected["apiVersion"]

    if "data" in expected:
        assert rehydrate_result["data"] == expected["data"]

    if "data_user_name" in expected:
        assert rehydrate_result["data"]["user"]["name"] == expected["data_user_name"]

    if "data_value" in expected:
        assert rehydrate_result["data"]["value"] == expected["data_value"]

    if "warnings_is_array" in expected:
        assert "warnings" in rehydrate_result, "result missing 'warnings' key"
        assert isinstance(rehydrate_result["warnings"], list)


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
        assert result["apiVersion"] == expected["apiVersion"]

    if "components" in expected:
        assert result["components"] == expected["components"]


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
        assert result["apiVersion"] == expected["apiVersion"]
    if "pointer" in expected:
        assert result["pointer"] == expected["pointer"]
    if "schema_is_object" in expected:
        assert isinstance(result["schema"], dict)
    if "dependency_count" in expected:
        assert result["dependencyCount"] == expected["dependency_count"]
    if "dependency_count_gte" in expected:
        assert result["dependencyCount"] >= expected["dependency_count_gte"]


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

    result = engine.convert_all_components(
        inp["schema"],
        inp.get("convert_options"),
        inp.get("extract_options"),
    )

    if "apiVersion" in expected:
        assert result["apiVersion"] == expected["apiVersion"]
    if "full_is_object" in expected:
        assert isinstance(result["full"], dict)
    if "components_count" in expected:
        assert len(result["components"]) == expected["components_count"]
