# frozen_string_literal: true

# TDD acceptance tests for Ruby engine exception hierarchy.

require "minitest/autorun"
require_relative "../lib/json_schema_llm_engine"

class TestExceptions < Minitest::Test
  def test_engine_error_hierarchy
    assert JsonSchemaLlmEngine::SchemaConversionError < JsonSchemaLlmEngine::EngineError
    assert JsonSchemaLlmEngine::RehydrationError < JsonSchemaLlmEngine::EngineError
    assert JsonSchemaLlmEngine::ResponseParsingError < JsonSchemaLlmEngine::EngineError
    assert JsonSchemaLlmEngine::EngineError < StandardError
  end

  def test_transport_error
    err = JsonSchemaLlmEngine::LlmTransportError.new("timeout", status_code: -1)
    assert_equal(-1, err.status_code)
    assert_includes err.message, "timeout"
    refute err.http_error?
  end

  def test_transport_error_http
    err = JsonSchemaLlmEngine::LlmTransportError.new("rate limited", status_code: 429)
    assert_equal 429, err.status_code
    assert err.http_error?
  end
end
