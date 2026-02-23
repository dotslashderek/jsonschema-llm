# frozen_string_literal: true

# Strategy interface for formatting LLM requests per provider.

module JsonSchemaLlmEngine
  # Duck-type interface for provider-specific request formatting.
  #
  # Each provider has its own request/response JSON shape.
  # Implementations handle the formatting and content extraction.
  #
  # Implementations must respond to:
  #   format(prompt, llm_schema, config) → LlmRequest
  #   extract_content(raw_response) → String
  #
  # @example
  #   class MyFormatter
  #     def format(prompt, llm_schema, config)
  #       # Build provider-specific request
  #     end
  #
  #     def extract_content(raw_response)
  #       # Extract generated content from response
  #     end
  #   end
  module ProviderFormatter
    # @param prompt [String] the user's natural language prompt
    # @param llm_schema [Hash] the converted LLM-compatible JSON Schema
    # @param config [ProviderConfig] provider endpoint and model configuration
    # @return [LlmRequest] a formatted request ready for transport
    def format(prompt, llm_schema, config)
      raise NotImplementedError
    end

    # @param raw_response [String] the raw response body from the LLM provider
    # @return [String] the extracted JSON content string
    # @raise [ResponseParsingError] if the response cannot be parsed
    def extract_content(raw_response)
      raise NotImplementedError
    end
  end
end
