# frozen_string_literal: true

# Consumer-provided SPI for executing LLM HTTP requests.

module JsonSchemaLlmEngine
  # Checked exception for transport failures.
  #
  # Transport failures are expected — callers must handle them
  # (retry, fallback, report).
  class LlmTransportError < StandardError
    # @return [Integer] HTTP status code, or -1 for non-HTTP failures
    attr_reader :status_code

    def initialize(message, status_code: -1)
      super(message)
      @status_code = status_code
    end

    # @return [Boolean] true if this represents an HTTP error
    def http_error?
      @status_code > 0
    end
  end

end
