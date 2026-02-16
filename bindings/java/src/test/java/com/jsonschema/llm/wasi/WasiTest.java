package com.jsonschema.llm.wasi;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

class WasiTest {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private JsonSchemaLlmWasi engine;

    @BeforeEach
    void setUp() {
        engine = new JsonSchemaLlmWasi();
    }

    @AfterEach
    void tearDown() {
        engine.close();
    }

    private ObjectNode simpleSchema() {
        ObjectNode schema = MAPPER.createObjectNode();
        schema.put("type", "object");

        ObjectNode properties = MAPPER.createObjectNode();
        ObjectNode nameProp = MAPPER.createObjectNode().put("type", "string");
        ObjectNode ageProp = MAPPER.createObjectNode().put("type", "integer").put("minimum", 0);
        properties.set("name", nameProp);
        properties.set("age", ageProp);
        schema.set("properties", properties);

        schema.set("required", MAPPER.createArrayNode().add("name").add("age"));
        return schema;
    }

    @Test
    void convertSimple() throws Exception {
        JsonNode result = engine.convert(simpleSchema());
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
        ObjectNode schema = simpleSchema();
        JsonNode convertResult = engine.convert(schema);
        JsonNode codec = convertResult.get("codec");

        ObjectNode data = MAPPER.createObjectNode();
        data.put("name", "Ada");
        data.put("age", 36);

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
        ObjectNode schema = MAPPER.createObjectNode();
        schema.put("type", "object");
        ObjectNode properties = MAPPER.createObjectNode();
        properties.set("x", MAPPER.createObjectNode().put("type", "number"));
        schema.set("properties", properties);

        for (int i = 0; i < 5; i++) {
            JsonNode result = engine.convert(schema);
            assertTrue(result.has("schema"));
        }
    }
}
