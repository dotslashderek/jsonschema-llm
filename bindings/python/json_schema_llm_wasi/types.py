"""Typed result dataclasses and options for json-schema-llm WASI bindings."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


# ---------------------------------------------------------------------------
# Nested Result Types
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ProviderCompatError:
    """Error indicating a target provider constraint violation."""

    error: str
    pointer: str | None = None
    message: str | None = None

    @classmethod
    def from_dict(cls, raw: dict) -> ProviderCompatError:
        return cls(
            error=raw["error"],
            pointer=raw.get("pointer"),
            message=raw.get("message"),
        )


@dataclass(frozen=True)
class RehydrateWarning:
    """Warning produced during schema rehydration."""

    msg: str

    @classmethod
    def from_dict(cls, raw: dict) -> RehydrateWarning:
        return cls(msg=raw["msg"])


@dataclass(frozen=True)
class ExtractedComponent:
    """A component extracted from a larger schema."""

    pointer: str
    name: str | None = None
    schema: dict | None = None
    codec: dict | None = None

    @classmethod
    def from_dict(cls, pointer: str, raw: dict) -> ExtractedComponent:
        return cls(
            pointer=pointer,
            name=raw.get("name"),
            schema=raw.get("schema"),
            codec=raw.get("codec"),
        )


@dataclass(frozen=True)
class ComponentError:
    """Error encountered when processing a specific component."""

    pointer: str
    error: str

    @classmethod
    def from_dict(cls, raw: dict) -> ComponentError:
        return cls(
            pointer=raw["pointer"],
            error=raw["error"],
        )


# ---------------------------------------------------------------------------
# Result types
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ConvertResult:
    """Typed result of a schema conversion operation."""

    api_version: str
    schema: dict
    codec: dict
    provider_compat_errors: list[ProviderCompatError] | None = None

    @classmethod
    def from_dict(cls, raw: dict) -> ConvertResult:
        errors = raw.get("providerCompatErrors")
        return cls(
            api_version=raw["apiVersion"],
            schema=raw["schema"],
            codec=raw["codec"],
            provider_compat_errors=[ProviderCompatError.from_dict(e) for e in errors] if errors else None,
        )


@dataclass(frozen=True)
class RehydrateResult:
    """Typed result of a rehydration operation."""

    api_version: str
    data: Any
    warnings: list[RehydrateWarning] = field(default_factory=list)

    @classmethod
    def from_dict(cls, raw: dict) -> RehydrateResult:
        return cls(
            api_version=raw["apiVersion"],
            data=raw["data"],
            warnings=[RehydrateWarning.from_dict(w) for w in raw.get("warnings", [])],
        )


@dataclass(frozen=True)
class ListComponentsResult:
    """Typed result of a list_components operation."""

    api_version: str
    components: list[str]

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
    components: dict[str, ExtractedComponent]
    component_errors: list[ComponentError] = field(default_factory=list)

    @classmethod
    def from_dict(cls, raw: dict) -> ConvertAllComponentsResult:
        comps = {}
        for item in raw.get("components", []):
            if isinstance(item, list) and len(item) == 2:
                pointer, comp_data = item
                comps[pointer] = ExtractedComponent.from_dict(pointer, comp_data)
            
        return cls(
            api_version=raw["apiVersion"],
            full=raw["full"],
            components=comps,
            component_errors=[ComponentError.from_dict(e) for e in raw.get("componentErrors", [])],
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
    """

    target: str | None = None
    mode: str | None = None
    max_depth: int | None = None
    recursion_limit: int | None = None
    polymorphism: str | None = None

    def to_dict(self) -> dict:
        """Serialize to a dict with kebab-case keys for the WASM ABI."""
        result = {}
        if self.target is not None:
            result["target"] = self.target
        if self.mode is not None:
            result["mode"] = self.mode
        if self.max_depth is not None:
            result["max-depth"] = self.max_depth
        if self.recursion_limit is not None:
            result["recursion-limit"] = self.recursion_limit
        if self.polymorphism is not None:
            result["polymorphism"] = self.polymorphism
        return result
