package com.jsonschema.llm.engine;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;

import java.util.HashMap;
import java.util.Map;

/**
 * Google Gemini generateContent API formatter.
 *
 * <p>
 * Formats requests using {@code generationConfig.responseMimeType} and
 * {@code generationConfig.responseSchema} for structured output.
 * Works with any endpoint that speaks the Gemini generateContent API
 * (Google AI, Vertex AI, etc.).
 *
 * <p>
 * API key is passed via the {@code x-goog-api-key} header (never in the URL).
 *
 * <p>
 * Extracts content from {@code candidates[0].content.parts[0].text}.
 */
public class GeminiFormatter implements ProviderFormatter {

    private static final ObjectMapper MAPPER = new ObjectMapper();

    @Override
    public LlmRequest format(String prompt, JsonNode llmSchema, ProviderConfig config) {
        try {
            ObjectNode requestBody = MAPPER.createObjectNode();

            // Contents: user message
            ArrayNode contents = requestBody.putArray("contents");
            ObjectNode content = contents.addObject();
            ArrayNode parts = content.putArray("parts");
            ObjectNode textPart = parts.addObject();
            textPart.put("text", prompt);

            // Generation config with response schema
            ObjectNode generationConfig = requestBody.putObject("generationConfig");
            generationConfig.put("responseMimeType", "application/json");
            generationConfig.set("responseSchema", llmSchema);

            String body = MAPPER.writeValueAsString(requestBody);

            Map<String, String> headers = new HashMap<>(config.headers());
            headers.putIfAbsent("Content-Type", "application/json");

            return new LlmRequest(config.url(), headers, body);
        } catch (JsonProcessingException e) {
            throw new EngineException.ResponseParsingException(
                    "Failed to serialize Gemini request", e);
        }
    }

    @Override
    public String extractContent(String rawResponse) {
        try {
            JsonNode root = MAPPER.readTree(rawResponse);

            JsonNode candidates = root.get("candidates");
            if (candidates == null || !candidates.isArray() || candidates.isEmpty()) {
                throw new EngineException.ResponseParsingException(
                        "Gemini response missing 'candidates' array or empty: "
                                + truncate(rawResponse, 200));
            }

            JsonNode firstCandidate = candidates.get(0);

            // Check for safety block
            JsonNode finishReason = firstCandidate.get("finishReason");
            if (finishReason != null && "SAFETY".equals(finishReason.asText())) {
                throw new EngineException.ResponseParsingException(
                        "Gemini response blocked by SAFETY filter: "
                                + truncate(rawResponse, 200));
            }

            JsonNode content = firstCandidate.get("content");
            if (content == null) {
                throw new EngineException.ResponseParsingException(
                        "Gemini response missing 'candidates[0].content': "
                                + truncate(rawResponse, 200));
            }

            JsonNode partsNode = content.get("parts");
            if (partsNode == null || !partsNode.isArray() || partsNode.isEmpty()) {
                throw new EngineException.ResponseParsingException(
                        "Gemini response missing 'candidates[0].content.parts': "
                                + truncate(rawResponse, 200));
            }

            JsonNode text = partsNode.get(0).get("text");
            if (text == null || !text.isTextual()) {
                throw new EngineException.ResponseParsingException(
                        "Gemini response 'candidates[0].content.parts[0].text' is missing or not text: "
                                + truncate(rawResponse, 200));
            }

            return text.asText();
        } catch (EngineException.ResponseParsingException e) {
            throw e;
        } catch (Exception e) {
            throw new EngineException.ResponseParsingException(
                    "Failed to parse Gemini response: " + truncate(rawResponse, 200), e);
        }
    }

    private static String truncate(String s, int maxLen) {
        if (s == null)
            return "<null>";
        return s.length() <= maxLen ? s : s.substring(0, maxLen) + "...";
    }
}
