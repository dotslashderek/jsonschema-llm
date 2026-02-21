package com.jsonschema.llm.engine;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;

import java.util.HashMap;
import java.util.Map;

/**
 * Anthropic Claude Messages API formatter (tool-use wire format).
 *
 * <p>
 * Formats requests using Claude's tool-use mechanism: defines a tool
 * whose {@code input_schema} is the LLM schema, then forces the model
 * to call it via {@code tool_choice: { type: "tool", name: "response" }}.
 * Works with any endpoint that speaks the Anthropic Messages API
 * (api.anthropic.com, AWS Bedrock, etc.).
 *
 * <p>
 * Extracts content from {@code content[].type == "tool_use" → input}.
 */
public class ClaudeFormatter implements ProviderFormatter {

    private static final ObjectMapper MAPPER = new ObjectMapper();

    @Override
    public LlmRequest format(String prompt, JsonNode llmSchema, ProviderConfig config) {
        try {
            ObjectNode requestBody = MAPPER.createObjectNode();
            requestBody.put("model", config.model());
            requestBody.put("max_tokens", 4096);

            // Messages array: single user message
            ArrayNode messages = requestBody.putArray("messages");
            ObjectNode userMessage = messages.addObject();
            userMessage.put("role", "user");
            userMessage.put("content", prompt);

            // Tool definition
            ArrayNode tools = requestBody.putArray("tools");
            ObjectNode tool = tools.addObject();
            tool.put("name", "response");
            tool.put("description", "Generate structured output matching the schema");
            tool.set("input_schema", llmSchema);

            // Force tool use
            ObjectNode toolChoice = requestBody.putObject("tool_choice");
            toolChoice.put("type", "tool");
            toolChoice.put("name", "response");

            String body = MAPPER.writeValueAsString(requestBody);

            Map<String, String> headers = new HashMap<>(config.headers());
            headers.putIfAbsent("Content-Type", "application/json");

            return new LlmRequest(config.url(), headers, body);
        } catch (JsonProcessingException e) {
            throw new EngineException.ResponseParsingException(
                    "Failed to serialize Claude request", e);
        }
    }

    @Override
    public String extractContent(String rawResponse) {
        try {
            JsonNode root = MAPPER.readTree(rawResponse);

            JsonNode content = root.get("content");
            if (content == null || !content.isArray()) {
                throw new EngineException.ResponseParsingException(
                        "Claude response missing 'content' array: "
                                + truncate(rawResponse, 200));
            }

            // Find the tool_use block
            for (JsonNode block : content) {
                if ("tool_use".equals(block.path("type").asText())) {
                    JsonNode input = block.get("input");
                    if (input != null) {
                        return MAPPER.writeValueAsString(input);
                    }
                }
            }

            throw new EngineException.ResponseParsingException(
                    "Claude response contains no 'tool_use' content block: "
                            + truncate(rawResponse, 200));
        } catch (EngineException.ResponseParsingException e) {
            throw e;
        } catch (Exception e) {
            throw new EngineException.ResponseParsingException(
                    "Failed to parse Claude response: " + truncate(rawResponse, 200), e);
        }
    }

    private static String truncate(String s, int maxLen) {
        if (s == null)
            return "<null>";
        return s.length() <= maxLen ? s : s.substring(0, maxLen) + "...";
    }
}
