package com.jsonschema.llm.engine;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;

import java.util.HashMap;
import java.util.Map;

/**
 * OpenAI Chat Completions API formatter.
 *
 * <p>
 * Formats requests using {@code response_format: { type: "json_schema" }}
 * for structured output. Works with any endpoint that speaks the Chat
 * Completions wire format (OpenAI, Azure OpenAI, etc.).
 *
 * <p>
 * Extracts content from {@code choices[0].message.content}.
 */
public class ChatCompletionsFormatter implements ProviderFormatter {

    private static final ObjectMapper MAPPER = new ObjectMapper();

    @Override
    public LlmRequest format(String prompt, JsonNode llmSchema, ProviderConfig config) {
        try {
            ObjectNode requestBody = MAPPER.createObjectNode();
            requestBody.put("model", config.model());

            // Messages array: single user message
            ArrayNode messages = requestBody.putArray("messages");
            ObjectNode userMessage = messages.addObject();
            userMessage.put("role", "user");
            userMessage.put("content", prompt);

            // Structured output: response_format with json_schema
            ObjectNode responseFormat = requestBody.putObject("response_format");
            responseFormat.put("type", "json_schema");

            ObjectNode jsonSchema = responseFormat.putObject("json_schema");
            jsonSchema.put("name", "response");
            jsonSchema.put("strict", true);
            jsonSchema.set("schema", llmSchema);

            String body = MAPPER.writeValueAsString(requestBody);

            Map<String, String> headers = new HashMap<>(config.headers());
            headers.putIfAbsent("Content-Type", "application/json");

            return new LlmRequest(config.url(), headers, body);
        } catch (JsonProcessingException e) {
            throw new EngineException.ResponseParsingException(
                    "Failed to serialize Chat Completions request", e);
        }
    }

    @Override
    public String extractContent(String rawResponse) {
        try {
            JsonNode root = MAPPER.readTree(rawResponse);

            JsonNode choices = root.get("choices");
            if (choices == null || !choices.isArray() || choices.isEmpty()) {
                throw new EngineException.ResponseParsingException(
                        "Chat Completions response missing 'choices' array or empty: "
                                + truncate(rawResponse, 200));
            }

            JsonNode message = choices.get(0).get("message");
            if (message == null) {
                throw new EngineException.ResponseParsingException(
                        "Chat Completions response missing 'choices[0].message': "
                                + truncate(rawResponse, 200));
            }

            JsonNode content = message.get("content");
            if (content == null || content.isNull()) {
                throw new EngineException.ResponseParsingException(
                        "Chat Completions response has null 'choices[0].message.content': "
                                + truncate(rawResponse, 200));
            }

            if (!content.isTextual()) {
                throw new EngineException.ResponseParsingException(
                        "Chat Completions response 'choices[0].message.content' is not a text node: "
                                + truncate(rawResponse, 200));
            }

            return content.asText();
        } catch (EngineException.ResponseParsingException e) {
            throw e;
        } catch (Exception e) {
            throw new EngineException.ResponseParsingException(
                    "Failed to parse Chat Completions response: " + truncate(rawResponse, 200), e);
        }
    }

    private static String truncate(String s, int maxLen) {
        if (s == null)
            return "<null>";
        return s.length() <= maxLen ? s : s.substring(0, maxLen) + "...";
    }
}
