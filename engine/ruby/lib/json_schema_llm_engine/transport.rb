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

  # Duck-type interface for consumer-provided HTTP transport.
  #
  # The engine formats the request; the consumer handles transport
  # (HTTP client, thread model, APM, debugging).
  #
  # Implementations must respond to:
  #   execute(request) → String
  #
  # @example
  #   class MyTransport
  #     def execute(request)
  #       # Make HTTP call, return raw response body
  #     end
  #   end
  module LlmTransport
    # @param request [LlmRequest] the formatted LLM request
    # @return [String] the raw response body string
    # @raise [LlmTransportError] if the transport fails
    def execute(request)
      raise NotImplementedError
    end
  end
end
