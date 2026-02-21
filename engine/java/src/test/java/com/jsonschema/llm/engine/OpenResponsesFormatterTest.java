package com.jsonschema.llm.engine;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;

import java.util.Map;

import static org.assertj.core.api.Assertions.*;

/**
 * Acceptance tests for OpenResponsesFormatter (Responses API / OpenResponses
 * wire format).
 */
class OpenResponsesFormatterTest {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private final OpenResponsesFormatter formatter = new OpenResponsesFormatter();

    // ---------------------------------------------------------------
    // 1. Format produces valid Responses API payload
    // ---------------------------------------------------------------

    @Test
    void format_producesValidResponsesPayload() throws Exception {
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
                "https://api.openai.com/v1/responses", "gpt-4o",
                Map.of("Authorization", "Bearer test-key"));

        LlmRequest request = formatter.format("Generate a name", llmSchema, config);

        assertThat(request.url()).isEqualTo("https://api.openai.com/v1/responses");
        assertThat(request.headers()).containsEntry("Authorization", "Bearer test-key");
        assertThat(request.headers()).containsEntry("Content-Type", "application/json");

        JsonNode body = MAPPER.readTree(request.body());
        assertThat(body.get("model").asText()).isEqualTo("gpt-4o");

        // Responses API uses "input" not "messages"
        assertThat(body.get("input").asText()).isEqualTo("Generate a name");

        // Structured output via text.format
        JsonNode textFormat = body.get("text").get("format");
        assertThat(textFormat.get("type").asText()).isEqualTo("json_schema");
        assertThat(textFormat.get("name").asText()).isEqualTo("response");
        assertThat(textFormat.get("schema")).isEqualTo(llmSchema);
    }

    // ---------------------------------------------------------------
    // 2. Extract content from valid output items
    // ---------------------------------------------------------------

    @Test
    void extractContent_parsesOutputText() {
        String response = """
                {
                  "id": "resp_123",
                  "output": [
                    {
                      "type": "message",
                      "content": [
                        {
                          "type": "output_text",
                          "text": "{\\"name\\": \\"Ada\\"}"
                        }
                      ]
                    }
                  ]
                }
                """;

        String content = formatter.extractContent(response);

        assertThat(content).isEqualTo("{\"name\": \"Ada\"}");
    }

    // ---------------------------------------------------------------
    // 3. Missing/empty output throws descriptive error
    // ---------------------------------------------------------------

    @Test
    void extractContent_noOutput_throwsError() {
        assertThatThrownBy(() -> formatter.extractContent("{\"output\": []}"))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("output");
    }

    @Test
    void extractContent_malformedResponse_throwsError() {
        assertThatThrownBy(() -> formatter.extractContent("NOT JSON"))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("Failed to parse");
    }
}
