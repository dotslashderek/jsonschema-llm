"""Anthropic Claude Messages API formatter (tool-use wire format)."""

from __future__ import annotations

import json
from typing import Any

from jsonschema_llm_engine.exceptions import ResponseParsingError
from jsonschema_llm_engine.types import LlmRequest, ProviderConfig


def _truncate(s: str | None, max_len: int = 200) -> str:
    if s is None:
        return "<null>"
    return s if len(s) <= max_len else s[:max_len] + "..."


class ClaudeFormatter:
    """Anthropic Claude Messages API formatter (tool-use wire format).

    Formats requests using Claude's tool-use mechanism: defines a tool
    whose ``input_schema`` is the LLM schema, then forces the model to
    call it via ``tool_choice: { type: "tool", name: "response" }``.

    Extracts content from ``content[].type == "tool_use" → input``.
    """

    def format(
        self, prompt: str, llm_schema: Any, config: ProviderConfig
    ) -> LlmRequest:
        request_body = {
            "model": config.model,
            "max_tokens": 4096,
            "messages": [{"role": "user", "content": prompt}],
            "tools": [
                {
                    "name": "response",
                    "description": "Generate structured output matching the schema",
                    "input_schema": llm_schema,
                }
            ],
            "tool_choice": {"type": "tool", "name": "response"},
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

            content = root.get("content")
            if not content or not isinstance(content, list):
                raise ResponseParsingError(
                    f"Claude response missing 'content' array: "
                    f"{_truncate(raw_response)}"
                )

            for block in content:
                if block.get("type") == "tool_use":
                    input_data = block.get("input")
                    if input_data is not None:
                        return json.dumps(input_data)

            raise ResponseParsingError(
                f"Claude response contains no 'tool_use' content block: "
                f"{_truncate(raw_response)}"
            )
        except ResponseParsingError:
            raise
        except Exception as e:
            raise ResponseParsingError(
                f"Failed to parse Claude response: {_truncate(raw_response)}"
            ) from e
