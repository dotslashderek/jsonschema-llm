# frozen_string_literal: true

require "minitest/autorun"
require_relative "lib/jsonschema_llm"

class JsonSchemaLlmTest < Minitest::Test
  def setup
    @engine = JsonSchemaLlm::Engine.new
  end

  def test_convert_simple
    schema = {
      "type" => "object",
      "properties" => {
        "name" => { "type" => "string" },
        "age" => { "type" => "integer", "minimum" => 0 }
      },
      "required" => %w[name age]
    }
    result = @engine.convert(schema)
    assert result.key?("apiVersion")
    assert result.key?("schema")
    assert result.key?("codec")
  end

  def test_convert_error
    assert_raises(JsonSchemaLlm::JslError) do
      @engine.send(:call_jsl, "jsl_convert", "NOT VALID JSON", "{}")
    end
  end

  def test_roundtrip
    schema = {
      "type" => "object",
      "properties" => {
        "name" => { "type" => "string" },
        "age" => { "type" => "integer", "minimum" => 0 }
      },
      "required" => %w[name age]
    }
    convert_result = @engine.convert(schema)

    data = { "name" => "Ada", "age" => 36 }
    rehydrate_result = @engine.rehydrate(data, convert_result["codec"], schema)

    assert rehydrate_result.key?("apiVersion")
    assert_equal "Ada", rehydrate_result["data"]["name"]
  end

  def test_rehydrate_error
    assert_raises(JsonSchemaLlm::JslError) do
      @engine.send(:call_jsl, "jsl_rehydrate",
                   '{"key":"value"}', "NOT VALID JSON",
                   '{"type":"object"}')
    end
  end

  def test_multiple_calls
    schema = { "type" => "object", "properties" => { "x" => { "type" => "number" } } }
    5.times do
      result = @engine.convert(schema)
      refute_nil result["schema"]
    end
  end
end
