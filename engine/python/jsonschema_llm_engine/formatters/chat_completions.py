"""OpenAI Chat Completions API formatter."""

from __future__ import annotations

import json
from typing import Any

from jsonschema_llm_engine.exceptions import ResponseParsingError
from jsonschema_llm_engine.types import LlmRequest, ProviderConfig


def _truncate(s: str | None, max_len: int = 200) -> str:
    if s is None:
        return "<null>"
    return s if len(s) <= max_len else s[:max_len] + "..."


class ChatCompletionsFormatter:
    """OpenAI Chat Completions API formatter.

    Formats requests using ``response_format: { type: "json_schema" }``
    for structured output. Works with any endpoint that speaks the Chat
    Completions wire format (OpenAI, Azure OpenAI, etc.).

    Extracts content from ``choices[0].message.content``.
    """

    def format(
        self, prompt: str, llm_schema: Any, config: ProviderConfig
    ) -> LlmRequest:
        request_body = {
            "model": config.model,
            "messages": [{"role": "user", "content": prompt}],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "response",
                    "strict": True,
                    "schema": llm_schema,
                },
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

            choices = root.get("choices")
            if not choices or not isinstance(choices, list):
                raise ResponseParsingError(
                    f"Chat Completions response missing 'choices' array: "
                    f"{_truncate(raw_response)}"
                )

            message = choices[0].get("message")
            if message is None:
                raise ResponseParsingError(
                    f"Chat Completions response missing 'choices[0].message': "
                    f"{_truncate(raw_response)}"
                )

            content = message.get("content")
            if content is None:
                raise ResponseParsingError(
                    f"Chat Completions response has null 'content': "
                    f"{_truncate(raw_response)}"
                )

            if not isinstance(content, str):
                raise ResponseParsingError(
                    f"Chat Completions 'content' is not a string: "
                    f"{_truncate(raw_response)}"
                )

            return content
        except ResponseParsingError:
            raise
        except Exception as e:
            raise ResponseParsingError(
                f"Failed to parse Chat Completions response: {_truncate(raw_response)}"
            ) from e
