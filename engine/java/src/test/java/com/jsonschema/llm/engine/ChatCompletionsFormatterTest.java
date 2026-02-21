package com.jsonschema.llm.engine;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;

import java.util.Map;

import static org.assertj.core.api.Assertions.*;

/**
 * Unit tests for ChatCompletionsFormatter (formerly OpenAiFormatter).
 */
class ChatCompletionsFormatterTest {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private final ChatCompletionsFormatter formatter = new ChatCompletionsFormatter();

    // ---------------------------------------------------------------
    // 1. Format produces valid Chat Completions payload
    // ---------------------------------------------------------------

    @Test
    void format_producesValidChatCompletionsPayload() throws Exception {
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
                "https://api.openai.com/v1/chat/completions", "gpt-4o",
                Map.of("Authorization", "Bearer test-key"));

        LlmRequest request = formatter.format("Generate a name", llmSchema, config);

        assertThat(request.url()).isEqualTo("https://api.openai.com/v1/chat/completions");
        assertThat(request.headers()).containsEntry("Authorization", "Bearer test-key");
        assertThat(request.headers()).containsEntry("Content-Type", "application/json");

        JsonNode body = MAPPER.readTree(request.body());
        assertThat(body.get("model").asText()).isEqualTo("gpt-4o");
        assertThat(body.get("messages").isArray()).isTrue();
        assertThat(body.get("messages").get(0).get("role").asText()).isEqualTo("user");
        assertThat(body.get("messages").get(0).get("content").asText()).isEqualTo("Generate a name");

        JsonNode responseFormat = body.get("response_format");
        assertThat(responseFormat.get("type").asText()).isEqualTo("json_schema");
        assertThat(responseFormat.get("json_schema").get("strict").asBoolean()).isTrue();
        assertThat(responseFormat.get("json_schema").get("schema")).isEqualTo(llmSchema);
    }

    // ---------------------------------------------------------------
    // 2. Extract content from valid response
    // ---------------------------------------------------------------

    @Test
    void extractContent_parsesChoicesMessageContent() {
        String response = """
                {
                  "choices": [{
                    "message": {
                      "content": "{\\"name\\": \\"Ada\\"}"
                    }
                  }]
                }
                """;

        String content = formatter.extractContent(response);

        assertThat(content).isEqualTo("{\"name\": \"Ada\"}");
    }

    // ---------------------------------------------------------------
    // 3. Malformed response throws descriptive error
    // ---------------------------------------------------------------

    @Test
    void extractContent_malformedResponse_throwsError() {
        assertThatThrownBy(() -> formatter.extractContent("NOT JSON"))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("Failed to parse Chat Completions response");
    }

    @Test
    void extractContent_missingChoices_throwsError() {
        assertThatThrownBy(() -> formatter.extractContent("{}"))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("missing 'choices'");
    }

    @Test
    void extractContent_emptyChoices_throwsError() {
        assertThatThrownBy(() -> formatter.extractContent("{\"choices\": []}"))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("missing 'choices'");
    }

    @Test
    void extractContent_nullContent_throwsError() {
        String response = """
                {
                  "choices": [{
                    "message": {
                      "content": null
                    }
                  }]
                }
                """;

        assertThatThrownBy(() -> formatter.extractContent(response))
                .isInstanceOf(EngineException.ResponseParsingException.class)
                .hasMessageContaining("null 'choices[0].message.content'");
    }
}
