"""Typed result dataclasses and options for json-schema-llm WASI bindings."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


# ---------------------------------------------------------------------------
# Result types
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ConvertResult:
    """Typed result of a schema conversion operation."""

    api_version: str
    schema: dict
    codec: dict
    provider_compat_errors: list[dict] | None = None

    @classmethod
    def from_dict(cls, raw: dict) -> ConvertResult:
        return cls(
            api_version=raw["apiVersion"],
            schema=raw["schema"],
            codec=raw["codec"],
            provider_compat_errors=raw.get("providerCompatErrors"),
        )


@dataclass(frozen=True)
class RehydrateResult:
    """Typed result of a rehydration operation."""

    api_version: str
    data: Any
    warnings: list[dict] = field(default_factory=list)

    @classmethod
    def from_dict(cls, raw: dict) -> RehydrateResult:
        return cls(
            api_version=raw["apiVersion"],
            data=raw["data"],
            warnings=raw.get("warnings", []),
        )


@dataclass(frozen=True)
class ListComponentsResult:
    """Typed result of a list_components operation."""

    api_version: str
    components: list[dict]

    @classmethod
    def from_dict(cls, raw: dict) -> ListComponentsResult:
        return cls(
            api_version=raw["apiVersion"],
            components=raw["components"],
        )


@dataclass(frozen=True)
class ExtractComponentResult:
    """Typed result of a single component extraction."""

    api_version: str
    schema: dict
    pointer: str
    dependency_count: int
    missing_refs: list[str] = field(default_factory=list)

    @classmethod
    def from_dict(cls, raw: dict) -> ExtractComponentResult:
        return cls(
            api_version=raw["apiVersion"],
            schema=raw["schema"],
            pointer=raw["pointer"],
            dependency_count=raw["dependencyCount"],
            missing_refs=raw.get("missingRefs", []),
        )


@dataclass(frozen=True)
class ConvertAllComponentsResult:
    """Typed result of converting all components in one call."""

    api_version: str
    full: dict
    components: list[dict]
    component_errors: list[dict] = field(default_factory=list)

    @classmethod
    def from_dict(cls, raw: dict) -> ConvertAllComponentsResult:
        return cls(
            api_version=raw["apiVersion"],
            full=raw["full"],
            components=raw["components"],
            component_errors=raw.get("componentErrors", []),
        )


# ---------------------------------------------------------------------------
# Options
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ConvertOptions:
    """Options for schema conversion, with WASM ABI key normalization.

    Usage::

        # Direct construction (idiomatic Python)
        opts = ConvertOptions(target="openai-strict", max_depth=50)

        # Builder pattern (fluent API)
        opts = (ConvertOptions.builder()
                .target("openai-strict")
                .max_depth(50)
                .build())
    """

    target: str | None = None
    mode: str | None = None
    max_depth: int | None = None
    recursion_limit: int | None = None
    polymorphism: str | None = None

    def to_dict(self) -> dict:
        """Serialize to a dict with kebab-case keys for the WASM ABI."""
        mapping = {
            "target": "target",
            "mode": "mode",
            "max_depth": "max-depth",
            "recursion_limit": "recursion-limit",
            "polymorphism": "polymorphism",
        }
        return {
            mapping[k]: v
            for k, v in {
                "target": self.target,
                "mode": self.mode,
                "max_depth": self.max_depth,
                "recursion_limit": self.recursion_limit,
                "polymorphism": self.polymorphism,
            }.items()
            if v is not None
        }

    @classmethod
    def builder(cls) -> ConvertOptionsBuilder:
        """Create a fluent builder for ConvertOptions."""
        return ConvertOptionsBuilder()


class ConvertOptionsBuilder:
    """Fluent builder for ConvertOptions."""

    def __init__(self) -> None:
        self._target: str | None = None
        self._mode: str | None = None
        self._max_depth: int | None = None
        self._recursion_limit: int | None = None
        self._polymorphism: str | None = None

    def target(self, value: str) -> ConvertOptionsBuilder:
        self._target = value
        return self

    def mode(self, value: str) -> ConvertOptionsBuilder:
        self._mode = value
        return self

    def max_depth(self, value: int) -> ConvertOptionsBuilder:
        self._max_depth = value
        return self

    def recursion_limit(self, value: int) -> ConvertOptionsBuilder:
        self._recursion_limit = value
        return self

    def polymorphism(self, value: str) -> ConvertOptionsBuilder:
        self._polymorphism = value
        return self

    def build(self) -> ConvertOptions:
        return ConvertOptions(
            target=self._target,
            mode=self._mode,
            max_depth=self._max_depth,
            recursion_limit=self._recursion_limit,
            polymorphism=self._polymorphism,
        )
