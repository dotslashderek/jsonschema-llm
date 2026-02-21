"""TDD acceptance tests for the Python engine formatters and types.

These tests are written BEFORE the implementations (TDD RED phase).
They encode the acceptance criteria from issue #186 and must not be
modified once committed unless the requirements change.
"""

import json
import pytest


# ── Types ────────────────────────────────────────────────────────────────────


class TestProviderConfig:
    def test_creation(self):
        from jsonschema_llm_engine.types import ProviderConfig

        config = ProviderConfig(
            url="https://api.openai.com/v1/chat/completions",
            model="gpt-4o",
            headers={"Authorization": "Bearer sk-test"},
        )
        assert config.url == "https://api.openai.com/v1/chat/completions"
        assert config.model == "gpt-4o"
        assert config.headers["Authorization"] == "Bearer sk-test"

    def test_empty_url_raises(self):
        from jsonschema_llm_engine.types import ProviderConfig

        with pytest.raises(ValueError):
            ProviderConfig(url="", model="gpt-4o")

    def test_empty_model_raises(self):
        from jsonschema_llm_engine.types import ProviderConfig

        with pytest.raises(ValueError):
            ProviderConfig(url="https://example.com", model="")


class TestLlmRequest:
    def test_creation(self):
        from jsonschema_llm_engine.types import LlmRequest

        req = LlmRequest(
            url="https://api.openai.com/v1/chat/completions",
            headers={"Content-Type": "application/json"},
            body='{"model": "gpt-4o"}',
        )
        assert req.url == "https://api.openai.com/v1/chat/completions"
        assert req.body == '{"model": "gpt-4o"}'

    def test_empty_url_raises(self):
        from jsonschema_llm_engine.types import LlmRequest

        with pytest.raises(ValueError):
            LlmRequest(url="", headers={}, body="{}")


class TestRoundtripResult:
    def test_is_valid_when_no_errors(self):
        from jsonschema_llm_engine.types import RoundtripResult

        result = RoundtripResult(
            data={"name": "test"},
            raw_llm_response={"choices": []},
            warnings=[],
            validation_errors=[],
        )
        assert result.is_valid is True

    def test_is_not_valid_when_errors(self):
        from jsonschema_llm_engine.types import RoundtripResult

        result = RoundtripResult(
            data={"name": "test"},
            raw_llm_response={"choices": []},
            warnings=[],
            validation_errors=["required property 'age' missing"],
        )
        assert result.is_valid is False


# ── Exceptions ───────────────────────────────────────────────────────────────


class TestExceptions:
    def test_engine_error_hierarchy(self):
        from jsonschema_llm_engine.exceptions import (
            EngineError,
            RehydrationError,
            ResponseParsingError,
            SchemaConversionError,
        )

        assert issubclass(SchemaConversionError, EngineError)
        assert issubclass(RehydrationError, EngineError)
        assert issubclass(ResponseParsingError, EngineError)

    def test_transport_error(self):
        from jsonschema_llm_engine.transport import LlmTransportError

        err = LlmTransportError("timeout", status_code=-1)
        assert err.status_code == -1
        assert "timeout" in str(err)


# ── Chat Completions Formatter ───────────────────────────────────────────────


class TestChatCompletionsFormatter:
    def test_format_produces_valid_request(self):
        from jsonschema_llm_engine.formatters.chat_completions import (
            ChatCompletionsFormatter,
        )
        from jsonschema_llm_engine.types import ProviderConfig

        formatter = ChatCompletionsFormatter()
        config = ProviderConfig(
            url="https://api.openai.com/v1/chat/completions",
            model="gpt-4o",
            headers={"Authorization": "Bearer sk-test"},
        )
        schema = {"type": "object", "properties": {"name": {"type": "string"}}}
        request = formatter.format("Generate a name", schema, config)

        assert request.url == "https://api.openai.com/v1/chat/completions"
        assert "Content-Type" in request.headers

        body = json.loads(request.body)
        assert body["model"] == "gpt-4o"
        assert body["messages"][0]["role"] == "user"
        assert body["messages"][0]["content"] == "Generate a name"
        assert body["response_format"]["type"] == "json_schema"
        assert body["response_format"]["json_schema"]["strict"] is True

    def test_extract_content_valid(self):
        from jsonschema_llm_engine.formatters.chat_completions import (
            ChatCompletionsFormatter,
        )

        formatter = ChatCompletionsFormatter()
        response = json.dumps(
            {"choices": [{"message": {"content": '{"name": "Alice"}'}}]}
        )
        content = formatter.extract_content(response)
        assert content == '{"name": "Alice"}'

    def test_extract_content_missing_choices_raises(self):
        from jsonschema_llm_engine.exceptions import ResponseParsingError
        from jsonschema_llm_engine.formatters.chat_completions import (
            ChatCompletionsFormatter,
        )

        formatter = ChatCompletionsFormatter()
        with pytest.raises(ResponseParsingError):
            formatter.extract_content("{}")


# ── OpenResponses Formatter ──────────────────────────────────────────────────


class TestOpenResponsesFormatter:
    def test_format_produces_valid_request(self):
        from jsonschema_llm_engine.formatters.open_responses import (
            OpenResponsesFormatter,
        )
        from jsonschema_llm_engine.types import ProviderConfig

        formatter = OpenResponsesFormatter()
        config = ProviderConfig(
            url="https://api.openai.com/v1/responses",
            model="gpt-4o",
            headers={},
        )
        schema = {"type": "object"}
        request = formatter.format("prompt", schema, config)

        body = json.loads(request.body)
        assert body["model"] == "gpt-4o"
        assert body["input"] == "prompt"
        assert body["text"]["format"]["type"] == "json_schema"

    def test_extract_content_valid(self):
        from jsonschema_llm_engine.formatters.open_responses import (
            OpenResponsesFormatter,
        )

        formatter = OpenResponsesFormatter()
        response = json.dumps(
            {
                "output": [
                    {
                        "type": "message",
                        "content": [{"type": "output_text", "text": '{"result": 42}'}],
                    }
                ]
            }
        )
        content = formatter.extract_content(response)
        assert content == '{"result": 42}'

    def test_extract_content_missing_output_raises(self):
        from jsonschema_llm_engine.exceptions import ResponseParsingError
        from jsonschema_llm_engine.formatters.open_responses import (
            OpenResponsesFormatter,
        )

        formatter = OpenResponsesFormatter()
        with pytest.raises(ResponseParsingError):
            formatter.extract_content("{}")


# ── Claude Formatter ─────────────────────────────────────────────────────────


class TestClaudeFormatter:
    def test_format_produces_valid_request(self):
        from jsonschema_llm_engine.formatters.claude import ClaudeFormatter
        from jsonschema_llm_engine.types import ProviderConfig

        formatter = ClaudeFormatter()
        config = ProviderConfig(
            url="https://api.anthropic.com/v1/messages",
            model="claude-sonnet-4-20250514",
            headers={"x-api-key": "sk-ant-test"},
        )
        schema = {"type": "object"}
        request = formatter.format("prompt", schema, config)

        body = json.loads(request.body)
        assert body["model"] == "claude-sonnet-4-20250514"
        assert body["max_tokens"] == 4096
        assert body["tools"][0]["name"] == "response"
        assert body["tool_choice"]["type"] == "tool"

    def test_extract_content_valid(self):
        from jsonschema_llm_engine.formatters.claude import ClaudeFormatter

        formatter = ClaudeFormatter()
        response = json.dumps(
            {"content": [{"type": "tool_use", "input": {"name": "Bob"}}]}
        )
        content = formatter.extract_content(response)
        parsed = json.loads(content)
        assert parsed["name"] == "Bob"

    def test_extract_content_no_tool_use_raises(self):
        from jsonschema_llm_engine.exceptions import ResponseParsingError
        from jsonschema_llm_engine.formatters.claude import ClaudeFormatter

        formatter = ClaudeFormatter()
        with pytest.raises(ResponseParsingError):
            formatter.extract_content(json.dumps({"content": [{"type": "text"}]}))


# ── Gemini Formatter ─────────────────────────────────────────────────────────


class TestGeminiFormatter:
    def test_format_produces_valid_request(self):
        from jsonschema_llm_engine.formatters.gemini import GeminiFormatter
        from jsonschema_llm_engine.types import ProviderConfig

        formatter = GeminiFormatter()
        config = ProviderConfig(
            url="https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:generateContent",
            model="gemini-pro",
            headers={"x-goog-api-key": "test-key"},
        )
        schema = {"type": "object"}
        request = formatter.format("prompt", schema, config)

        body = json.loads(request.body)
        assert body["contents"][0]["parts"][0]["text"] == "prompt"
        assert body["generationConfig"]["responseMimeType"] == "application/json"
        assert body["generationConfig"]["responseSchema"] == schema

    def test_extract_content_valid(self):
        from jsonschema_llm_engine.formatters.gemini import GeminiFormatter

        formatter = GeminiFormatter()
        response = json.dumps(
            {"candidates": [{"content": {"parts": [{"text": '{"val": 1}'}]}}]}
        )
        content = formatter.extract_content(response)
        assert content == '{"val": 1}'

    def test_extract_content_safety_block_raises(self):
        from jsonschema_llm_engine.exceptions import ResponseParsingError
        from jsonschema_llm_engine.formatters.gemini import GeminiFormatter

        formatter = GeminiFormatter()
        response = json.dumps(
            {"candidates": [{"finishReason": "SAFETY", "content": {}}]}
        )
        with pytest.raises(ResponseParsingError):
            formatter.extract_content(response)

    def test_extract_content_missing_candidates_raises(self):
        from jsonschema_llm_engine.exceptions import ResponseParsingError
        from jsonschema_llm_engine.formatters.gemini import GeminiFormatter

        formatter = GeminiFormatter()
        with pytest.raises(ResponseParsingError):
            formatter.extract_content("{}")


# ── Transport Protocol ───────────────────────────────────────────────────────


class TestTransportProtocol:
    def test_transport_protocol_exists(self):
        from jsonschema_llm_engine.transport import LlmTransport

        # Protocol should be importable — structural subtyping
        assert hasattr(LlmTransport, "execute")

    def test_formatter_protocol_exists(self):
        from jsonschema_llm_engine.formatter import ProviderFormatter

        assert hasattr(ProviderFormatter, "format")
        assert hasattr(ProviderFormatter, "extract_content")
