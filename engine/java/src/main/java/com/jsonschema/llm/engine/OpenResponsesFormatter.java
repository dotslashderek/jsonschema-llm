package com.jsonschema.llm.engine;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;

import java.util.HashMap;
import java.util.Map;

/**
 * OpenResponses / Responses API formatter.
 *
 * <p>
 * Formats requests using the Responses API wire format
 * ({@code text.format: { type: "json_schema" }}) for structured output.
 * Works with any endpoint implementing the OpenResponses standard
 * (OpenAI Responses API, Ollama, vLLM, etc.).
 *
 * <p>
 * Extracts content from
 * {@code output[].content[].type == "output_text" → text}.
 */
public class OpenResponsesFormatter implements ProviderFormatter {

    private static final ObjectMapper MAPPER = new ObjectMapper();

    @Override
    public LlmRequest format(String prompt, JsonNode llmSchema, ProviderConfig config) {
        try {
            ObjectNode requestBody = MAPPER.createObjectNode();
            requestBody.put("model", config.model());

            // Responses API uses "input" (string) instead of "messages" array
            requestBody.put("input", prompt);

            // Structured output: text.format with json_schema
            ObjectNode text = requestBody.putObject("text");
            ObjectNode format = text.putObject("format");
            format.put("type", "json_schema");
            format.put("name", "response");
            format.set("schema", llmSchema);

            String body = MAPPER.writeValueAsString(requestBody);

            Map<String, String> headers = new HashMap<>(config.headers());
            headers.putIfAbsent("Content-Type", "application/json");

            return new LlmRequest(config.url(), headers, body);
        } catch (JsonProcessingException e) {
            throw new EngineException.ResponseParsingException(
                    "Failed to serialize OpenResponses request", e);
        }
    }

    @Override
    public String extractContent(String rawResponse) {
        try {
            JsonNode root = MAPPER.readTree(rawResponse);

            JsonNode output = root.get("output");
            if (output == null || !output.isArray() || output.isEmpty()) {
                throw new EngineException.ResponseParsingException(
                        "OpenResponses response missing 'output' array or empty: "
                                + truncate(rawResponse, 200));
            }

            // Find the first "message" item
            for (JsonNode item : output) {
                if ("message".equals(item.path("type").asText())) {
                    JsonNode content = item.get("content");
                    if (content != null && content.isArray()) {
                        for (JsonNode part : content) {
                            if ("output_text".equals(part.path("type").asText())) {
                                JsonNode text = part.get("text");
                                if (text != null && text.isTextual()) {
                                    return text.asText();
                                }
                            }
                        }
                    }
                }
            }

            throw new EngineException.ResponseParsingException(
                    "OpenResponses response has no 'output_text' in output items: "
                            + truncate(rawResponse, 200));
        } catch (EngineException.ResponseParsingException e) {
            throw e;
        } catch (Exception e) {
            throw new EngineException.ResponseParsingException(
                    "Failed to parse OpenResponses response: " + truncate(rawResponse, 200), e);
        }
    }

    private static String truncate(String s, int maxLen) {
        if (s == null)
            return "<null>";
        return s.length() <= maxLen ? s : s.substring(0, maxLen) + "...";
    }
}
