"""Consumer-provided SPI for executing LLM HTTP requests."""

from __future__ import annotations

from typing import Protocol, runtime_checkable

from jsonschema_llm_engine.types import LlmRequest


class LlmTransportError(Exception):
    """Checked exception for transport failures.

    Transport failures are expected — callers must handle them
    (retry, fallback, report).

    Args:
        message: Human-readable error description.
        status_code: HTTP status code, or -1 for non-HTTP failures.
    """

    def __init__(self, message: str, status_code: int = -1) -> None:
        super().__init__(message)
        self.status_code = status_code

    @property
    def is_http_error(self) -> bool:
        """True if this represents an HTTP error (status code > 0)."""
        return self.status_code > 0


@runtime_checkable
class LlmTransport(Protocol):
    """Consumer-provided SPI for executing LLM HTTP requests.

    The engine formats the request; the consumer handles transport
    (HTTP client, thread model, APM, debugging). This decouples the
    engine from any specific HTTP library.
    """

    def execute(self, request: LlmRequest) -> str:
        """Execute an LLM request and return the raw response body.

        Args:
            request: The formatted LLM request.

        Returns:
            The raw response body string.

        Raises:
            LlmTransportError: If the transport fails.
        """
        ...
