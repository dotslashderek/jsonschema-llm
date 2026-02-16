# frozen_string_literal: true

# Conformance fixture tests for the WASI-backed jsonschema-llm wrapper.
#
# Loads fixtures from tests/conformance/fixtures.json and runs each fixture
# through the appropriate engine method, asserting expected outcomes.

require "minitest/autorun"
require "json"
require_relative "lib/jsonschema_llm"

FIXTURES_PATH = File.join(__dir__, "..", "..", "tests", "conformance", "fixtures.json")
FIXTURES = JSON.parse(File.read(FIXTURES_PATH))

class ConformanceConvertTest < Minitest::Test
  def setup
    @engine = JsonSchemaLlm::Engine.new
  end

  FIXTURES["suites"]["convert"]["fixtures"].each do |fx|
    define_method("test_#{fx['id']}") do
      input = fx["input"]
      expected = fx["expected"]

      # Error case: schema_raw → raw FFI
      if input.key?("schema_raw")
        assert expected["is_error"], "schema_raw fixture must have is_error=true"

        opts_json = JSON.generate(input.fetch("options", {}))
        err = assert_raises(JsonSchemaLlm::JslError) do
          @engine.send(:call_jsl, "jsl_convert", input["schema_raw"], opts_json)
        end

        if expected.key?("error_has_keys")
          expected["error_has_keys"].each do |key|
            case key
            when "code"
              refute_nil err.code, "error missing 'code'"
            when "message"
              refute_nil err.message, "error missing 'message'"
            end
          end
        end

        if expected.key?("error_code")
          assert_equal expected["error_code"], err.code
        end
        return
      end

      # Normal convert
      result = @engine.convert(input["schema"], input.fetch("options", {}))
      assert_convert_expected(result, expected)
    end
  end

  private

  def assert_convert_expected(result, expected)
    if expected.key?("has_keys")
      expected["has_keys"].each do |key|
        assert result.key?(key), "result missing key '#{key}'"
      end
    end

    if expected.key?("apiVersion")
      assert_equal expected["apiVersion"], result["apiVersion"]
    end

    if expected.key?("schema_has_properties")
      assert result["schema"].key?("properties"), "schema missing 'properties'"
    end

    if expected.key?("codec_has_schema_uri")
      refute_nil result["codec"], "codec is nil"
    end
  end
end

class ConformanceRoundtripTest < Minitest::Test
  def setup
    @engine = JsonSchemaLlm::Engine.new
  end

  FIXTURES["suites"]["roundtrip"]["fixtures"].each do |fx|
    define_method("test_#{fx['id']}") do
      input = fx["input"]
      expected = fx["expected"]

      convert_result = @engine.convert(input["schema"], input.fetch("options", {}))
      rehydrate_result = @engine.rehydrate(
        input["data"], convert_result["codec"], input["schema"]
      )

      if expected.key?("has_keys")
        expected["has_keys"].each do |key|
          assert rehydrate_result.key?(key), "result missing key '#{key}'"
        end
      end

      if expected.key?("apiVersion")
        assert_equal expected["apiVersion"], rehydrate_result["apiVersion"]
      end

      if expected.key?("data")
        assert_equal expected["data"], rehydrate_result["data"]
      end

      if expected.key?("data_user_name")
        assert_equal expected["data_user_name"],
                     rehydrate_result["data"]["user"]["name"]
      end

      if expected.key?("data_value")
        assert_equal expected["data_value"], rehydrate_result["data"]["value"]
      end

      if expected.key?("warnings_is_array")
        warnings = rehydrate_result.fetch("warnings", [])
        assert_kind_of Array, warnings
      end
    end
  end
end

class ConformanceRehydrateErrorTest < Minitest::Test
  def setup
    @engine = JsonSchemaLlm::Engine.new
  end

  FIXTURES["suites"]["rehydrate_error"]["fixtures"].each do |fx|
    define_method("test_#{fx['id']}") do
      input = fx["input"]
      expected = fx["expected"]

      assert expected["is_error"], "rehydrate_error fixture must have is_error=true"

      data_json = JSON.generate(input["data"])
      schema_json = JSON.generate(input["schema"])
      codec_arg = input.fetch("codec_raw", "{}")

      err = assert_raises(JsonSchemaLlm::JslError) do
        @engine.send(:call_jsl, "jsl_rehydrate", data_json, codec_arg, schema_json)
      end

      if expected.key?("error_has_keys")
        expected["error_has_keys"].each do |key|
          case key
          when "code"
            refute_nil err.code, "error missing 'code'"
          when "message"
            refute_nil err.message, "error missing 'message'"
          end
        end
      end
    end
  end
end
