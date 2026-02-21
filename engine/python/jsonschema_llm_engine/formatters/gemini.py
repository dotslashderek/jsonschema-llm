"""Google Gemini generateContent API formatter."""

from __future__ import annotations

import json
from typing import Any

from jsonschema_llm_engine.exceptions import ResponseParsingError
from jsonschema_llm_engine.types import LlmRequest, ProviderConfig


def _truncate(s: str | None, max_len: int = 200) -> str:
    if s is None:
        return "<null>"
    return s if len(s) <= max_len else s[:max_len] + "..."


class GeminiFormatter:
    """Google Gemini generateContent API formatter.

    Formats requests using ``generationConfig.responseMimeType`` and
    ``generationConfig.responseSchema`` for structured output.

    API key is passed via the ``x-goog-api-key`` header (never in the URL).

    Extracts content from ``candidates[0].content.parts[0].text``.
    """

    def format(
        self, prompt: str, llm_schema: Any, config: ProviderConfig
    ) -> LlmRequest:
        request_body = {
            "contents": [{"parts": [{"text": prompt}]}],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": llm_schema,
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

            candidates = root.get("candidates")
            if not candidates or not isinstance(candidates, list):
                raise ResponseParsingError(
                    f"Gemini response missing 'candidates' array: "
                    f"{_truncate(raw_response)}"
                )

            first = candidates[0]

            # Check for safety block
            finish_reason = first.get("finishReason")
            if finish_reason == "SAFETY":
                raise ResponseParsingError(
                    f"Gemini response blocked by SAFETY filter: "
                    f"{_truncate(raw_response)}"
                )

            content = first.get("content")
            if content is None:
                raise ResponseParsingError(
                    f"Gemini response missing 'candidates[0].content': "
                    f"{_truncate(raw_response)}"
                )

            parts = content.get("parts")
            if not parts or not isinstance(parts, list):
                raise ResponseParsingError(
                    f"Gemini response missing 'candidates[0].content.parts': "
                    f"{_truncate(raw_response)}"
                )

            text = parts[0].get("text")
            if text is None or not isinstance(text, str):
                raise ResponseParsingError(
                    f"Gemini 'parts[0].text' is missing or not text: "
                    f"{_truncate(raw_response)}"
                )

            return text
        except ResponseParsingError:
            raise
        except Exception as e:
            raise ResponseParsingError(
                f"Failed to parse Gemini response: {_truncate(raw_response)}"
            ) from e
