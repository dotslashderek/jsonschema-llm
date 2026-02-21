package com.jsonschema.llm.engine;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;

import java.util.Map;

import static org.assertj.core.api.Assertions.*;

/**
 * Acceptance tests for ClaudeFormatter (Anthropic Messages API — tool use wire
 * format).
 */
class ClaudeFormatterTest {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private final ClaudeFormatter formatter = new ClaudeFormatter();

    // ---------------------------------------------------------------
    // 1. Format produces valid Claude tool-use payload
    // ---------------------------------------------------------------

    @Test
    void format_producesValidClaudePayload() throws Exception {
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
                "https://api.anthropic.com/v1/messages", "claude-sonnet-4-20250514",
                Map.of("x-api-key", "test-key", "anthropic-version", "2023-06-01"));

        LlmRequest request = formatter.format("Generate a name", llmSchema, config);

        assertThat(request.url()).isEqualTo("https://api.anthropic.com/v1/messages");
        assertThat(request.headers()).containsEntry("x-api-key", "test-key");
        assertThat(request.headers()).containsEntry("anthropic-version", "2023-06-01");
        assertThat(request.headers()).containsEntry("Content-Type", "application/json");

        JsonNode body = MAPPER.readTree(request.body());
        assertThat(body.get("model").asText()).isEqualTo("claude-sonnet-4-20250514");

        // Messages
        assertThat(body.get("messages").isArray()).isTrue();
        assertThat(body.get("messages").get(0).get("role").asText()).isEqualTo("user");
        assertThat(body.get("messages").get(0).get("content").asText()).isEqualTo("Generate a name");

        // Tool definition
        JsonNode tools = body.get("tools");
        assertThat(tools.isArray()).isTrue();
        assertThat(tools.get(0).get("name").asText()).isEqualTo("response");
        assertThat(tools.get(0).get("input_schema")).isEqualTo(llmSchema);

        // Tool choice forces structured output
        JsonNode toolChoice = body.get("tool_choice");
        assertThat(toolChoice.get("type").asText()).isEqualTo("tool");
        assertThat(toolChoice.get("name").asText()).isEqualTo("response");
    }

    // ---------------------------------------------------------------
    // 2. Extract content from valid tool_use response
    // ---------------------------------------------------------------

    @Test
    void extractContent_parsesToolUseContent() throws Exception {
        String response = """
                {
                  "content": [
                    {
                      "type": "tool_use",
                      "id": "toolu_123",
                      "name": "response",
                      "input": {"name": "Ada"}
                    }
                  ],
                  "stop_reason": "tool_use"
                }
                """;

        String content = formatter.extractContent(response);

        // Should return the JSON-stringified input
        JsonNode parsed = MAPPER.readTree(content);
        assertThat(parsed.get("name").asText()).isEqualTo("Ada");
    }

    // ---------------------------------------------------------------
    // 3. Missing tool_use throws descriptive error
    // ---------------------------------------------------------------

    @Test
    void extractContent_noToolUse_throwsError() {
        String response = """
                {
                  "content": [
                    {
                      "type": "text",
                      "text": "I cannot do that."
                    }
                  ],
                  "stop_reason": "end_turn"
                }
                """;

        assertThatThrownBy(() -> formatter.extractContent(response))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("tool_use");
    }

    @Test
    void extractContent_malformedResponse_throwsError() {
        assertThatThrownBy(() -> formatter.extractContent("NOT JSON"))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("Failed to parse");
    }

    // ---------------------------------------------------------------
    // 4. Refusal (end_turn without tool_use)
    // ---------------------------------------------------------------

    @Test
    void extractContent_refusalResponse_throwsError() {
        String response = """
                {
                  "content": [],
                  "stop_reason": "end_turn"
                }
                """;

        assertThatThrownBy(() -> formatter.extractContent(response))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("tool_use");
    }
}
