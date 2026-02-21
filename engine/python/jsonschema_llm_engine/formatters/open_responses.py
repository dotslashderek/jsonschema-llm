"""OpenResponses / Responses API formatter."""

from __future__ import annotations

import json
from typing import Any

from jsonschema_llm_engine.exceptions import ResponseParsingError
from jsonschema_llm_engine.types import LlmRequest, ProviderConfig


def _truncate(s: str | None, max_len: int = 200) -> str:
    if s is None:
        return "<null>"
    return s if len(s) <= max_len else s[:max_len] + "..."


class OpenResponsesFormatter:
    """OpenResponses / Responses API formatter.

    Formats requests using ``text.format: { type: "json_schema" }``
    for structured output. Works with any endpoint implementing the
    OpenResponses standard (OpenAI Responses API, Ollama, vLLM, etc.).

    Extracts content from ``output[].content[].type == "output_text" → text``.
    """

    def format(
        self, prompt: str, llm_schema: Any, config: ProviderConfig
    ) -> LlmRequest:
        request_body = {
            "model": config.model,
            "input": prompt,
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "response",
                    "schema": llm_schema,
                }
            },
        }
        headers = {**config.headers, "Content-Type": "application/json"}
        return LlmRequest(
            url=config.url,
            headers=headers,
            body=json.dumps(request_body),
        )

    def extract_content(self, raw_response: str) -> str:
        try:
            root = json.loads(raw_response)

            output = root.get("output")
            if not output or not isinstance(output, list):
                raise ResponseParsingError(
                    f"OpenResponses response missing 'output' array: "
                    f"{_truncate(raw_response)}"
                )

            for item in output:
                if item.get("type") == "message":
                    content = item.get("content")
                    if content and isinstance(content, list):
                        for part in content:
                            if part.get("type") == "output_text":
                                text = part.get("text")
                                if text and isinstance(text, str):
                                    return text

            raise ResponseParsingError(
                f"OpenResponses response has no 'output_text': "
                f"{_truncate(raw_response)}"
            )
        except ResponseParsingError:
            raise
        except Exception as e:
            raise ResponseParsingError(
                f"Failed to parse OpenResponses response: {_truncate(raw_response)}"
            ) from e
