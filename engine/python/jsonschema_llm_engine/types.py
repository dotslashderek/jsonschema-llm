"""Core types for the jsonschema-llm Python engine."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True)
class ProviderConfig:
    """Configuration for an LLM provider endpoint.

    Args:
        url: The provider API endpoint URL.
        model: The model identifier (e.g., "gpt-4o", "gemini-pro").
        headers: Additional HTTP headers (e.g., Authorization).
    """

    url: str
    model: str
    headers: dict[str, str] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if not self.url or not self.url.strip():
            raise ValueError("url must not be empty")
        if not self.model or not self.model.strip():
            raise ValueError("model must not be empty")


@dataclass(frozen=True)
class LlmRequest:
    """Immutable request to send to an LLM provider.

    Args:
        url: The provider endpoint URL.
        headers: HTTP headers (e.g., Authorization, Content-Type).
        body: The serialized request body (JSON string).
    """

    url: str
    headers: dict[str, str] = field(default_factory=dict)
    body: str = ""

    def __post_init__(self) -> None:
        if not self.url or not self.url.strip():
            raise ValueError("url must not be empty")


@dataclass(frozen=True)
class RoundtripResult:
    """Result of a full LLM roundtrip: convert → call LLM → rehydrate → validate.

    Args:
        data: The rehydrated, validated output matching the original schema shape.
        raw_llm_response: The raw response from the LLM provider (for debugging/audit).
        warnings: Advisory warnings from the rehydration step.
        validation_errors: JSON Schema validation errors against the original schema.
    """

    data: Any
    raw_llm_response: Any
    warnings: list[str] = field(default_factory=list)
    validation_errors: list[str] = field(default_factory=list)

    @property
    def is_valid(self) -> bool:
        """True if the rehydrated data passes JSON Schema validation."""
        return len(self.validation_errors) == 0
