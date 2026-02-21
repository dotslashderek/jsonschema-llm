package com.jsonschema.llm.engine;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;

import java.util.Map;

import static org.assertj.core.api.Assertions.*;

/**
 * Acceptance tests for GeminiFormatter (Google generateContent API wire
 * format).
 */
class GeminiFormatterTest {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private final GeminiFormatter formatter = new GeminiFormatter();

    // ---------------------------------------------------------------
    // 1. Format produces valid Gemini generateContent payload
    // ---------------------------------------------------------------

    @Test
    void format_producesValidGeminiPayload() throws Exception {
        JsonNode llmSchema = MAPPER.readTree("""
                {
                  "type": "object",
                  "properties": {
                    "name": { "type": "string" }
                  },
                  "required": ["name"]
                }
                """);

        ProviderConfig config = new ProviderConfig(
                "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent",
                "gemini-2.5-flash",
                Map.of("x-goog-api-key", "test-key"));

        LlmRequest request = formatter.format("Generate a name", llmSchema, config);

        assertThat(request.url())
                .isEqualTo("https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent");
        assertThat(request.headers()).containsEntry("x-goog-api-key", "test-key");
        assertThat(request.headers()).containsEntry("Content-Type", "application/json");

        JsonNode body = MAPPER.readTree(request.body());

        // Contents with user message
        JsonNode contents = body.get("contents");
        assertThat(contents.isArray()).isTrue();
        assertThat(contents.get(0).get("parts").get(0).get("text").asText()).isEqualTo("Generate a name");

        // Generation config with response schema
        JsonNode genConfig = body.get("generationConfig");
        assertThat(genConfig.get("responseMimeType").asText()).isEqualTo("application/json");
        assertThat(genConfig.get("responseSchema")).isEqualTo(llmSchema);
    }

    // ---------------------------------------------------------------
    // 2. Extract content from valid candidates response
    // ---------------------------------------------------------------

    @Test
    void extractContent_parsesCandidateText() {
        String response = """
                {
                  "candidates": [
                    {
                      "content": {
                        "parts": [
                          {
                            "text": "{\\"name\\": \\"Ada\\"}"
                          }
                        ]
                      },
                      "finishReason": "STOP"
                    }
                  ]
                }
                """;

        String content = formatter.extractContent(response);

        assertThat(content).isEqualTo("{\"name\": \"Ada\"}");
    }

    // ---------------------------------------------------------------
    // 3. Missing/empty candidates throws descriptive error
    // ---------------------------------------------------------------

    @Test
    void extractContent_noCandidates_throwsError() {
        assertThatThrownBy(() -> formatter.extractContent("{\"candidates\": []}"))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("candidates");
    }

    @Test
    void extractContent_malformedResponse_throwsError() {
        assertThatThrownBy(() -> formatter.extractContent("NOT JSON"))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("Failed to parse");
    }

    // ---------------------------------------------------------------
    // 4. Safety block (SAFETY finishReason)
    // ---------------------------------------------------------------

    @Test
    void extractContent_safetyBlock_throwsError() {
        String response = """
                {
                  "candidates": [
                    {
                      "finishReason": "SAFETY",
                      "safetyRatings": [
                        {"category": "HARM_CATEGORY_DANGEROUS_CONTENT", "probability": "HIGH"}
                      ]
                    }
                  ]
                }
                """;

        assertThatThrownBy(() -> formatter.extractContent(response))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("SAFETY");
    }
}
