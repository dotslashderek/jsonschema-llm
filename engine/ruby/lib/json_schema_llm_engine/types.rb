# frozen_string_literal: true

# Core types for the json-schema-llm Ruby engine.

module JsonSchemaLlmEngine
  # Configuration for an LLM provider endpoint.
  #
  # @!attribute [r] url
  #   @return [String] the provider API endpoint URL
  # @!attribute [r] model
  #   @return [String] the model identifier (e.g., "gpt-4o")
  # @!attribute [r] headers
  #   @return [Hash<String, String>] additional HTTP headers
  ProviderConfig = Struct.new(:url, :model, :headers, keyword_init: true) do
    def initialize(url:, model:, headers: {})
      raise ArgumentError, "url must not be empty" if url.nil? || url.strip.empty?
      raise ArgumentError, "model must not be empty" if model.nil? || model.strip.empty?

      super(url: url, model: model, headers: headers)
      freeze
    end
  end

  # Immutable request to send to an LLM provider.
  #
  # @!attribute [r] url
  #   @return [String] the provider endpoint URL
  # @!attribute [r] headers
  #   @return [Hash<String, String>] HTTP headers
  # @!attribute [r] body
  #   @return [String] the serialized request body (JSON string)
  LlmRequest = Struct.new(:url, :headers, :body, keyword_init: true) do
    def initialize(url:, headers: {}, body: "")
      raise ArgumentError, "url must not be empty" if url.nil? || url.strip.empty?

      super(url: url, headers: headers, body: body)
      freeze
    end
  end

  # Result of a full LLM roundtrip: convert → call LLM → rehydrate → validate.
  #
  # @!attribute [r] data
  #   @return [Object] the rehydrated, validated output
  # @!attribute [r] raw_llm_response
  #   @return [Object] the raw response from the LLM provider
  # @!attribute [r] warnings
  #   @return [Array<String>] advisory warnings from rehydration
  # @!attribute [r] validation_errors
  #   @return [Array<String>] JSON Schema validation errors (empty = valid)
  RoundtripResult = Struct.new(:data, :raw_llm_response, :warnings, :validation_errors, keyword_init: true) do
    def initialize(data:, raw_llm_response:, warnings: [], validation_errors: [])
      super(
        data: data,
        raw_llm_response: raw_llm_response,
        warnings: warnings,
        validation_errors: validation_errors
      )
      freeze
    end

    # @return [Boolean] true if the rehydrated data passes JSON Schema validation
    def valid?
      validation_errors.empty?
    end
  end
end
