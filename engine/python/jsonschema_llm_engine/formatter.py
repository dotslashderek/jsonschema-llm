"""Strategy interface for formatting LLM requests per provider."""

from __future__ import annotations

from typing import Any, Protocol, runtime_checkable

from jsonschema_llm_engine.types import LlmRequest, ProviderConfig


@runtime_checkable
class ProviderFormatter(Protocol):
    """Strategy interface for formatting LLM requests per provider.

    Each provider has its own request/response JSON shape.
    Implementations handle the formatting and content extraction.
    """

    def format(
        self, prompt: str, llm_schema: Any, config: ProviderConfig
    ) -> LlmRequest:
        """Format a prompt and LLM-compatible schema into a provider-specific request.

        Args:
            prompt: The user's natural language prompt.
            llm_schema: The converted LLM-compatible JSON Schema.
            config: Provider endpoint and model configuration.

        Returns:
            A formatted LlmRequest ready for transport.
        """
        ...

    def extract_content(self, raw_response: str) -> str:
        """Extract the generated content from a raw LLM response.

        Args:
            raw_response: The raw response body from the LLM provider.

        Returns:
            The extracted JSON content string.

        Raises:
            ResponseParsingError: If the response cannot be parsed.
        """
        ...
