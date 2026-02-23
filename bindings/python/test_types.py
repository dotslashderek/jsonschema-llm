"""Pure-unit tests for typed results and ConvertOptions (no WASM needed)."""

import pytest

from json_schema_llm_wasi.types import (
    ConvertAllComponentsResult,
    ConvertOptions,
    ConvertResult,
    ExtractComponentResult,
    ListComponentsResult,
    RehydrateResult,
)


# ---------------------------------------------------------------------------
# ConvertResult
# ---------------------------------------------------------------------------


class TestConvertResult:
    def test_from_dict_happy_path(self):
        raw = {
            "apiVersion": "1",
            "schema": {"type": "object"},
            "codec": {"transforms": []},
        }
        result = ConvertResult.from_dict(raw)
        assert result.api_version == "1"
        assert result.schema == {"type": "object"}
        assert result.codec == {"transforms": []}
        assert result.provider_compat_errors is None

    def test_from_dict_with_compat_errors(self):
        raw = {
            "apiVersion": "1",
            "schema": {},
            "codec": {},
            "providerCompatErrors": [{"error": "strict"}],
        }
        result = ConvertResult.from_dict(raw)
        assert result.provider_compat_errors == [{"error": "strict"}]

    def test_from_dict_missing_required_field(self):
        with pytest.raises(KeyError):
            ConvertResult.from_dict({"apiVersion": "1", "schema": {}})

    def test_frozen(self):
        result = ConvertResult(api_version="1", schema={}, codec={})
        with pytest.raises(AttributeError):
            result.schema = {"mutated": True}  # type: ignore[misc]


# ---------------------------------------------------------------------------
# RehydrateResult
# ---------------------------------------------------------------------------


class TestRehydrateResult:
    def test_from_dict_happy_path(self):
        raw = {
            "apiVersion": "1",
            "data": {"name": "Ada"},
        }
        result = RehydrateResult.from_dict(raw)
        assert result.api_version == "1"
        assert result.data == {"name": "Ada"}
        assert result.warnings == []

    def test_from_dict_with_warnings(self):
        raw = {
            "apiVersion": "1",
            "data": {},
            "warnings": [{"msg": "coerced"}],
        }
        result = RehydrateResult.from_dict(raw)
        assert result.warnings == [{"msg": "coerced"}]

    def test_from_dict_missing_data(self):
        with pytest.raises(KeyError):
            RehydrateResult.from_dict({"apiVersion": "1"})


# ---------------------------------------------------------------------------
# ListComponentsResult
# ---------------------------------------------------------------------------


class TestListComponentsResult:
    def test_from_dict_happy_path(self):
        raw = {
            "apiVersion": "1",
            "components": [
                {"pointer": "#/$defs/Pet", "name": "Pet"},
            ],
        }
        result = ListComponentsResult.from_dict(raw)
        assert result.api_version == "1"
        assert len(result.components) == 1
        assert result.components[0]["name"] == "Pet"


# ---------------------------------------------------------------------------
# ExtractComponentResult
# ---------------------------------------------------------------------------


class TestExtractComponentResult:
    def test_from_dict_happy_path(self):
        raw = {
            "apiVersion": "1",
            "schema": {"type": "object"},
            "pointer": "#/$defs/Pet",
            "dependencyCount": 3,
        }
        result = ExtractComponentResult.from_dict(raw)
        assert result.pointer == "#/$defs/Pet"
        assert result.dependency_count == 3
        assert result.missing_refs == []

    def test_from_dict_with_missing_refs(self):
        raw = {
            "apiVersion": "1",
            "schema": {},
            "pointer": "#/$defs/Foo",
            "dependencyCount": 0,
            "missingRefs": ["#/$defs/Bar"],
        }
        result = ExtractComponentResult.from_dict(raw)
        assert result.missing_refs == ["#/$defs/Bar"]


# ---------------------------------------------------------------------------
# ConvertAllComponentsResult
# ---------------------------------------------------------------------------


class TestConvertAllComponentsResult:
    def test_from_dict_happy_path(self):
        raw = {
            "apiVersion": "1",
            "full": {"schema": {}, "codec": {}},
            "components": [{"name": "Pet", "schema": {}, "codec": {}}],
        }
        result = ConvertAllComponentsResult.from_dict(raw)
        assert result.api_version == "1"
        assert len(result.components) == 1
        assert result.component_errors == []

    def test_from_dict_with_errors(self):
        raw = {
            "apiVersion": "1",
            "full": {},
            "components": [],
            "componentErrors": [{"pointer": "#/$defs/Broken", "error": "fail"}],
        }
        result = ConvertAllComponentsResult.from_dict(raw)
        assert len(result.component_errors) == 1


# ---------------------------------------------------------------------------
# ConvertOptions
# ---------------------------------------------------------------------------


class TestConvertOptions:
    def test_kwargs_construction(self):
        opts = ConvertOptions(target="openai-strict", max_depth=50)
        assert opts.target == "openai-strict"
        assert opts.max_depth == 50
        assert opts.mode is None

    def test_to_dict_kebab_case(self):
        opts = ConvertOptions(
            target="openai-strict",
            max_depth=50,
            recursion_limit=10,
        )
        d = opts.to_dict()
        assert d == {
            "target": "openai-strict",
            "max-depth": 50,
            "recursion-limit": 10,
        }

    def test_to_dict_omits_none(self):
        opts = ConvertOptions(target="gemini")
        d = opts.to_dict()
        assert d == {"target": "gemini"}
        assert "mode" not in d
        assert "max-depth" not in d

    def test_builder_fluent(self):
        opts = (
            ConvertOptions.builder()
            .target("openai-strict")
            .max_depth(50)
            .polymorphism("anyOf")
            .build()
        )
        assert opts.target == "openai-strict"
        assert opts.max_depth == 50
        assert opts.polymorphism == "anyOf"

    def test_frozen(self):
        opts = ConvertOptions(target="x")
        with pytest.raises(AttributeError):
            opts.target = "y"  # type: ignore[misc]
