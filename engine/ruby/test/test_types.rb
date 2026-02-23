# frozen_string_literal: true

# TDD acceptance tests for Ruby engine types.
# Written BEFORE implementations (RED phase).

require "minitest/autorun"
require_relative "../lib/json_schema_llm_engine"

class TestProviderConfig < Minitest::Test
  def test_creation
    config = JsonSchemaLlmEngine::ProviderConfig.new(
      url: "https://api.openai.com/v1/chat/completions",
      model: "gpt-4o",
      headers: { "Authorization" => "Bearer sk-test" }
    )
    assert_equal "https://api.openai.com/v1/chat/completions", config.url
    assert_equal "gpt-4o", config.model
    assert_equal "Bearer sk-test", config.headers["Authorization"]
  end

  def test_frozen
    config = JsonSchemaLlmEngine::ProviderConfig.new(
      url: "https://example.com", model: "test"
    )
    assert config.frozen?
  end

  def test_empty_url_raises
    assert_raises(ArgumentError) do
      JsonSchemaLlmEngine::ProviderConfig.new(url: "", model: "gpt-4o")
    end
  end

  def test_empty_model_raises
    assert_raises(ArgumentError) do
      JsonSchemaLlmEngine::ProviderConfig.new(url: "https://example.com", model: "")
    end
  end

  def test_default_headers
    config = JsonSchemaLlmEngine::ProviderConfig.new(
      url: "https://example.com", model: "gpt-4o"
    )
    assert_equal({}, config.headers)
  end
end

class TestLlmRequest < Minitest::Test
  def test_creation
    req = JsonSchemaLlmEngine::LlmRequest.new(
      url: "https://api.openai.com/v1/chat/completions",
      headers: { "Content-Type" => "application/json" },
      body: '{"model": "gpt-4o"}'
    )
    assert_equal "https://api.openai.com/v1/chat/completions", req.url
    assert_equal '{"model": "gpt-4o"}', req.body
  end

  def test_frozen
    req = JsonSchemaLlmEngine::LlmRequest.new(
      url: "https://example.com", headers: {}, body: "{}"
    )
    assert req.frozen?
  end

  def test_empty_url_raises
    assert_raises(ArgumentError) do
      JsonSchemaLlmEngine::LlmRequest.new(url: "", headers: {}, body: "{}")
    end
  end
end

class TestRoundtripResult < Minitest::Test
  def test_valid_when_no_errors
    result = JsonSchemaLlmEngine::RoundtripResult.new(
      data: { "name" => "test" },
      raw_llm_response: { "choices" => [] },
      warnings: [],
      validation_errors: []
    )
    assert result.valid?
  end

  def test_not_valid_when_errors
    result = JsonSchemaLlmEngine::RoundtripResult.new(
      data: { "name" => "test" },
      raw_llm_response: { "choices" => [] },
      warnings: [],
      validation_errors: ["required property 'age' missing"]
    )
    refute result.valid?
  end

  def test_frozen
    result = JsonSchemaLlmEngine::RoundtripResult.new(
      data: {}, raw_llm_response: {}, warnings: [], validation_errors: []
    )
    assert result.frozen?
  end
end
