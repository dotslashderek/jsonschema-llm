# frozen_string_literal: true

# TDD acceptance tests for the LlmRoundtripEngine.
# Construction/WASI tests are skipped when wasmtime gem is not available
# (e.g., local dev on Ruby < 3.1). They run in CI on Ruby 3.3.

require "minitest/autorun"
require "json"
require_relative "../lib/json_schema_llm_engine"

HAS_WASMTIME = begin
  require "wasmtime"
  true
rescue LoadError
  false
end

HAS_WASM_BINARY = HAS_WASMTIME && (
  ENV["JSON_SCHEMA_LLM_WASM_PATH"] && File.file?(ENV["JSON_SCHEMA_LLM_WASM_PATH"])
) || File.file?(
  File.join(__dir__, "..", "..", "..", "target", "wasm32-wasip1", "release", "json_schema_llm_wasi.wasm")
)

# Minimal mock formatter for testing the orchestration flow.
class MockFormatter
  attr_reader :format_calls, :extract_calls

  def initialize(extract_response: '{"name": "Alice"}')
    @format_calls = []
    @extract_calls = []
    @extract_response = extract_response
  end

  def format(prompt, llm_schema, config)
    @format_calls << { prompt: prompt, llm_schema: llm_schema, config: config }
    JsonSchemaLlmEngine::LlmRequest.new(
      url: config.url,
      headers: config.headers.merge("Content-Type" => "application/json"),
      body: JSON.generate({ model: config.model, prompt: prompt })
    )
  end

  def extract_content(raw_response)
    @extract_calls << raw_response
    @extract_response
  end
end

# Minimal mock transport for testing.
class MockTransport
  attr_reader :execute_calls

  def initialize(response: nil)
    @execute_calls = []
    @response = response || JSON.generate(
      { "choices" => [{ "message" => { "content" => '{"name": "Alice"}' } }] }
    )
  end

  def execute(request)
    @execute_calls << request
    @response
  end
end

class TestLlmRoundtripEngineValidation < Minitest::Test
  # Duck-type validation tests — no WASI needed

  def test_rejects_invalid_formatter
    skip "wasmtime gem required" unless HAS_WASMTIME && HAS_WASM_BINARY

    config = JsonSchemaLlmEngine::ProviderConfig.new(
      url: "https://example.com", model: "test"
    )
    assert_raises(ArgumentError) do
      JsonSchemaLlmEngine::LlmRoundtripEngine.new(
        formatter: Object.new,
        config: config,
        transport: MockTransport.new
      )
    end
  end

  def test_rejects_invalid_transport
    skip "wasmtime gem required" unless HAS_WASMTIME && HAS_WASM_BINARY

    config = JsonSchemaLlmEngine::ProviderConfig.new(
      url: "https://example.com", model: "test"
    )
    assert_raises(ArgumentError) do
      JsonSchemaLlmEngine::LlmRoundtripEngine.new(
        formatter: MockFormatter.new,
        config: config,
        transport: Object.new
      )
    end
  end

  def test_accepts_valid_dependencies
    skip "wasmtime gem + WASM binary required" unless HAS_WASMTIME && HAS_WASM_BINARY

    config = JsonSchemaLlmEngine::ProviderConfig.new(
      url: "https://example.com", model: "test"
    )
    engine = JsonSchemaLlmEngine::LlmRoundtripEngine.new(
      formatter: MockFormatter.new,
      config: config,
      transport: MockTransport.new
    )
    assert_instance_of JsonSchemaLlmEngine::LlmRoundtripEngine, engine
    engine.close
  end
end

class TestLlmRoundtripEngineGenerateWithPreconverted < Minitest::Test
  def test_calls_formatter_then_transport
    skip "wasmtime gem + WASM binary required" unless HAS_WASMTIME && HAS_WASM_BINARY

    formatter = MockFormatter.new
    transport = MockTransport.new
    config = JsonSchemaLlmEngine::ProviderConfig.new(
      url: "https://api.openai.com/v1/chat/completions",
      model: "gpt-4o",
      headers: { "Authorization" => "Bearer sk-test" }
    )
    engine = JsonSchemaLlmEngine::LlmRoundtripEngine.new(
      formatter: formatter,
      config: config,
      transport: transport
    )

    schema = { "$schema" => "https://json-schema.org/draft/2020-12/schema", "type" => "object", "properties" => { "name" => { "type" => "string" } } }

    result = engine.generate_with_preconverted(
      original_schema_json: JSON.generate(schema),
      codec_json: JSON.generate({
        "$schema" => "https://json-schema-llm.dev/codec/v1",
        "transforms" => [],
        "droppedConstraints" => []
      }),
      llm_schema: schema,
      prompt: "Generate a person"
    )

    # Formatter was called with prompt, schema, config
    assert_equal 1, formatter.format_calls.length
    assert_equal "Generate a person", formatter.format_calls[0][:prompt]

    # Transport was called
    assert_equal 1, transport.execute_calls.length

    # Extract was called
    assert_equal 1, formatter.extract_calls.length

    # Result has the right shape
    assert_instance_of JsonSchemaLlmEngine::RoundtripResult, result
    assert_instance_of Hash, result.data
    assert_instance_of Hash, result.raw_llm_response
    assert_instance_of Array, result.warnings
    assert_instance_of Array, result.validation_errors

    engine.close
  end
end
