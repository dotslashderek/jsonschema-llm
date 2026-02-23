# frozen_string_literal: true

# OpenAI Chat Completions formatter — formats requests for the
# /v1/chat/completions endpoint with structured output (json_schema).

require "json"

module JsonSchemaLlmEngine
  module Formatters
    class ChatCompletions
      # Format a prompt and schema into an OpenAI Chat Completions request.
      #
      # @param prompt [String] the user's natural language prompt
      # @param llm_schema [Hash] the LLM-compatible JSON Schema
      # @param config [ProviderConfig] provider endpoint/model configuration
      # @return [LlmRequest]
      def format(prompt, llm_schema, config)
        body = {
          "model" => config.model,
          "messages" => [
            { "role" => "user", "content" => prompt }
          ],
          "response_format" => {
            "type" => "json_schema",
            "json_schema" => {
              "name" => "response",
              "strict" => true,
              "schema" => llm_schema
            }
          }
        }

        headers = config.headers.merge("Content-Type" => "application/json")

        LlmRequest.new(
          url: config.url,
          headers: headers,
          body: JSON.generate(body)
        )
      end

      # Extract the generated content from an OpenAI Chat Completions response.
      #
      # @param raw_response [String] the raw response body
      # @return [String] the extracted JSON content
      # @raise [ResponseParsingError] if the response cannot be parsed
      def extract_content(raw_response)
        parsed = JSON.parse(raw_response)
        choices = parsed["choices"]

        unless choices.is_a?(Array) && !choices.empty?
          raise ResponseParsingError, "Missing or empty 'choices' in response"
        end

        content = choices.dig(0, "message", "content")

        unless content.is_a?(String) && !content.empty?
          raise ResponseParsingError, "Missing 'content' in first choice message"
        end

        content
      rescue JSON::ParserError => e
        raise ResponseParsingError, "Failed to parse response JSON: #{e.message}"
      end
    end
  end
end
