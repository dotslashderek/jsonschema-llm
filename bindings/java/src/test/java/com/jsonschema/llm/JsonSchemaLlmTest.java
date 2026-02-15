package com.jsonschema.llm;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

public class JsonSchemaLlmTest {

    private final ObjectMapper mapper = new ObjectMapper();

    @Test
    public void testConvertSimpleSchema() throws Exception {
        String schemaJson = """
                {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "age": { "type": "integer" }
                    },
                    "required": ["name"]
                }
                """;
        JsonNode schema = mapper.readTree(schemaJson);
        ConvertOptions options = ConvertOptions.builder()
                .target(ConvertOptions.Target.OPENAI_STRICT)
                .mode(ConvertOptions.Mode.STRICT)
                .build();

        ConvertResult result = JsonSchemaLlm.convert(schema, options);

        assertNotNull(result);
        assertEquals("1.0", result.apiVersion());
        assertTrue(result.schema().isObject());
        assertTrue(result.codec().isObject());

        // OpenAI strict mode requires additionalProperties: false
        assertTrue(result.schema().has("additionalProperties"));
        assertFalse(result.schema().get("additionalProperties").asBoolean());
    }

    @Test
    public void testRehydrate() throws Exception {
        String schemaJson = """
                {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    }
                }
                """;
        JsonNode schema = mapper.readTree(schemaJson);
        ConvertOptions options = ConvertOptions.builder().build();

        ConvertResult result = JsonSchemaLlm.convert(schema, options);

        // LLM output
        String llmOutputJson = """
                { "name": "Alice" }
                """;
        JsonNode llmOutput = mapper.readTree(llmOutputJson);

        RehydrateResult rehydrated = JsonSchemaLlm.rehydrate(llmOutput, result.codec(), schema);

        assertNotNull(rehydrated);
        assertEquals("1.0", rehydrated.apiVersion());
        assertEquals("Alice", rehydrated.data().get("name").asText());
    }

    @Test
    public void testInvalidSchemaError() throws Exception {
        // Invalid JSON schema (broken ref)
        String schemaJson = """
                {
                    "$ref": "#/definitions/missing"
                }
                """;
        JsonNode schema = mapper.readTree(schemaJson);
        ConvertOptions options = ConvertOptions.builder().build();

        // Expect exception
        JsonSchemaLlmException exception = assertThrows(JsonSchemaLlmException.class, () -> {
            JsonSchemaLlm.convert(schema, options);
        });
    }
}
