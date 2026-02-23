# frozen_string_literal: true

# Exception hierarchy for the json-schema-llm Ruby engine.

module JsonSchemaLlmEngine
  # Base error for all engine failures.
  class EngineError < StandardError; end

  # Raised when schema conversion via WASI fails.
  class SchemaConversionError < EngineError; end

  # Raised when rehydration via WASI fails.
  class RehydrationError < EngineError; end

  # Raised when parsing an LLM response fails.
  class ResponseParsingError < EngineError; end
end
