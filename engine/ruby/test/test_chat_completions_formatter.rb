# frozen_string_literal: true

# TDD acceptance tests for the ChatCompletionsFormatter.

require "minitest/autorun"
require "json"
require_relative "../lib/json_schema_llm_engine"

class TestChatCompletionsFormatter < Minitest::Test
  def setup
    @formatter = JsonSchemaLlmEngine::Formatters::ChatCompletions.new
  end

  def test_format_produces_valid_request
    config = JsonSchemaLlmEngine::ProviderConfig.new(
      url: "https://api.openai.com/v1/chat/completions",
      model: "gpt-4o",
      headers: { "Authorization" => "Bearer sk-test" }
    )
    schema = { "type" => "object", "properties" => { "name" => { "type" => "string" } } }
    request = @formatter.format("Generate a name", schema, config)

    assert_equal "https://api.openai.com/v1/chat/completions", request.url
    assert_includes request.headers.keys, "Content-Type"

    body = JSON.parse(request.body)
    assert_equal "gpt-4o", body["model"]
    assert_equal "user", body["messages"][0]["role"]
    assert_equal "Generate a name", body["messages"][0]["content"]
    assert_equal "json_schema", body["response_format"]["type"]
    assert_equal true, body["response_format"]["json_schema"]["strict"]
  end

  def test_extract_content_valid
    response = JSON.generate(
      { "choices" => [{ "message" => { "content" => '{"name": "Alice"}' } }] }
    )
    content = @formatter.extract_content(response)
    assert_equal '{"name": "Alice"}', content
  end

  def test_extract_content_missing_choices_raises
    assert_raises(JsonSchemaLlmEngine::ResponseParsingError) do
      @formatter.extract_content("{}")
    end
  end
end
