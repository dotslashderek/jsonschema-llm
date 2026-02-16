package com.jsonschema.llm.wasi;

import com.fasterxml.jackson.databind.JsonNode;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.Map;

import static org.junit.jupiter.api.Assertions.*;

class WasiTest {

    private JsonSchemaLlmWasi engine;

    @BeforeEach
    void setUp() {
        engine = new JsonSchemaLlmWasi();
    }

    @AfterEach
    void tearDown() {
        engine.close();
    }

    @Test
    void convertSimple() throws Exception {
        var schema = Map.of(
                "type", "object",
                "properties", Map.of(
                        "name", Map.of("type", "string"),
                        "age", Map.of("type", "integer", "minimum", 0)),
                "required", new String[] { "name", "age" });
        JsonNode result = engine.convert(schema);
        assertTrue(result.has("apiVersion"));
        assertTrue(result.has("schema"));
        assertTrue(result.has("codec"));
    }

    @Test
    void convertError() {
        assertThrows(JsonSchemaLlmWasi.JslException.class, () -> engine.callJsl("jsl_convert", "NOT VALID JSON", "{}"));
    }

    @Test
    void roundtrip() throws Exception {
        var schema = Map.of(
                "type", "object",
                "properties", Map.of(
                        "name", Map.of("type", "string"),
                        "age", Map.of("type", "integer", "minimum", 0)),
                "required", new String[] { "name", "age" });
        JsonNode convertResult = engine.convert(schema);
        JsonNode codec = convertResult.get("codec");

        var data = Map.of("name", "Ada", "age", 36);
        JsonNode rehydrated = engine.rehydrate(data, codec, schema);

        assertTrue(rehydrated.has("apiVersion"));
        assertEquals("Ada", rehydrated.get("data").get("name").asText());
    }

    @Test
    void rehydrateError() {
        assertThrows(JsonSchemaLlmWasi.JslException.class, () -> engine.callJsl("jsl_rehydrate",
                "{\"key\":\"value\"}", "NOT VALID JSON", "{\"type\":\"object\"}"));
    }

    @Test
    void multipleCalls() throws Exception {
        var schema = Map.of(
                "type", "object",
                "properties", Map.of("x", Map.of("type", "number")));
        for (int i = 0; i < 5; i++) {
            JsonNode result = engine.convert(schema);
            assertTrue(result.has("schema"));
        }
    }
}
