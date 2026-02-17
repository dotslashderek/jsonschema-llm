package com.jsonschema.llm.wasi;

import static org.junit.jupiter.api.Assertions.*;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;

import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

/**
 * Acceptance tests for the typed API surface: ConvertResult, RehydrateResult,
 * ConvertOptions builder, and the convertTyped/rehydrateTyped methods on
 * JsonSchemaLlmWasi.
 */
class TypedApiTest {

  private static final ObjectMapper MAPPER = new ObjectMapper();
  private static JsonSchemaLlmWasi engine;

  @BeforeAll
  static void setUp() throws Exception {
    String wasmPath = System.getenv("JSL_WASM_PATH");
    if (wasmPath != null && !wasmPath.isEmpty()) {
      engine = new JsonSchemaLlmWasi(wasmPath);
    } else {
      engine = new JsonSchemaLlmWasi();
    }
  }

  // ---------------------------------------------------------------
  // ConvertResult record tests
  // ---------------------------------------------------------------

  @Test
  void convertReturnsTypedResult() throws Exception {
    JsonNode schema = MAPPER.readTree("""
        {
          "type": "object",
          "properties": {
            "name": { "type": "string" },
            "age": { "type": "integer" }
          },
          "required": ["name"]
        }
        """);

    ConvertResult result = engine.convertTyped(schema);

    assertNotNull(result, "ConvertResult should not be null");
    assertNotNull(result.apiVersion(), "apiVersion should be present");
    assertFalse(result.apiVersion().isEmpty(), "apiVersion should not be empty");
    assertNotNull(result.schema(), "schema should not be null");
    assertNotNull(result.codec(), "codec should not be null");
    assertTrue(result.schema().isObject(), "schema should be an object");
  }

  @Test
  void convertResultFromJsonFactory() throws Exception {
    ObjectNode raw = MAPPER.createObjectNode();
    raw.put("apiVersion", "1.0");
    raw.set("schema", MAPPER.readTree("{\"type\":\"object\"}"));
    raw.set("codec", MAPPER.readTree("{\"transforms\":[]}"));
    raw.putNull("providerCompatErrors");

    ConvertResult result = ConvertResult.fromJson(raw);

    assertEquals("1.0", result.apiVersion());
    assertNotNull(result.schema());
    assertNotNull(result.codec());
  }

  @Test
  void convertResultIncludesProviderCompatErrors() throws Exception {
    ObjectNode raw = MAPPER.createObjectNode();
    raw.put("apiVersion", "1.0");
    raw.set("schema", MAPPER.readTree("{\"type\":\"object\"}"));
    raw.set("codec", MAPPER.readTree("{\"transforms\":[]}"));
    raw.set("providerCompatErrors", MAPPER.readTree("[{\"code\":\"test\"}]"));

    ConvertResult result = ConvertResult.fromJson(raw);

    assertNotNull(result.providerCompatErrors());
    assertTrue(result.providerCompatErrors().isArray());
    assertEquals(1, result.providerCompatErrors().size());
  }

  // ---------------------------------------------------------------
  // RehydrateResult record tests
  // ---------------------------------------------------------------

  @Test
  void rehydrateReturnsTypedResult() throws Exception {
    // First, convert to get codec
    JsonNode schema = MAPPER.readTree("""
        {
          "type": "object",
          "properties": {
            "name": { "type": "string" }
          },
          "required": ["name"]
        }
        """);

    ConvertResult convertResult = engine.convertTyped(schema);

    // Then rehydrate with simple data
    JsonNode data = MAPPER.readTree("{\"name\": \"Ada\"}");
    RehydrateResult result = engine.rehydrateTyped(data, convertResult.codec(), schema);

    assertNotNull(result, "RehydrateResult should not be null");
    assertNotNull(result.apiVersion(), "apiVersion should be present");
    assertNotNull(result.data(), "data should not be null");
    assertNotNull(result.warnings(), "warnings should not be null");
  }

  @Test
  void rehydrateResultFromJsonFactory() throws Exception {
    ObjectNode raw = MAPPER.createObjectNode();
    raw.put("apiVersion", "1.0");
    raw.set("data", MAPPER.readTree("{\"name\":\"Ada\"}"));
    raw.set("warnings", MAPPER.readTree("[]"));

    RehydrateResult result = RehydrateResult.fromJson(raw);

    assertEquals("1.0", result.apiVersion());
    assertEquals("Ada", result.data().get("name").asText());
    assertTrue(result.warnings().isArray());
    assertEquals(0, result.warnings().size());
  }

  // ---------------------------------------------------------------
  // ConvertOptions builder tests
  // ---------------------------------------------------------------

  @Test
  void convertWithOptions() throws Exception {
    JsonNode schema = MAPPER.readTree("""
        {
          "type": "object",
          "properties": {
            "name": { "type": "string" }
          }
        }
        """);

    ConvertOptions opts = ConvertOptions.builder()
        .target("openai-strict")
        .maxDepth(50)
        .recursionLimit(3)
        .polymorphism("any-of")
        .build();

    ConvertResult result = engine.convertTyped(schema, opts);

    assertNotNull(result);
    assertNotNull(result.schema());
    assertNotNull(result.codec());
  }

  @Test
  void convertOptionsBuilderGeneratesCorrectJson() throws Exception {
    ConvertOptions opts = ConvertOptions.builder()
        .target("openai-strict")
        .maxDepth(100)
        .recursionLimit(5)
        .polymorphism("flatten")
        .mode("permissive")
        .build();

    String json = opts.toJson();
    JsonNode parsed = MAPPER.readTree(json);

    assertEquals("openai-strict", parsed.get("target").asText());
    assertEquals(100, parsed.get("max-depth").asInt());
    assertEquals(5, parsed.get("recursion-limit").asInt());
    assertEquals("flatten", parsed.get("polymorphism").asText());
    assertEquals("permissive", parsed.get("mode").asText());
  }

  @Test
  void convertOptionsKeyNormalization() throws Exception {
    ConvertOptions opts = ConvertOptions.builder()
        .option("removeDefaults", true)
        .option("someOtherOption", "value")
        .build();

    String json = opts.toJson();
    JsonNode parsed = MAPPER.readTree(json);

    assertTrue(parsed.has("remove-defaults"), "camelCase should be normalized to kebab-case");
    assertTrue(parsed.has("some-other-option"), "camelCase should be normalized to kebab-case");
  }

  @Test
  void convertOptionsBuilderEmptyBuild() throws Exception {
    ConvertOptions opts = ConvertOptions.builder().build();

    String json = opts.toJson();
    assertEquals("{}", json, "Empty builder should produce empty JSON object");
  }

  @Test
  void convertOptionsBuilderRejectsNegativeMaxDepth() {
    assertThrows(IllegalArgumentException.class, () -> {
      ConvertOptions.builder().maxDepth(-1).build();
    });
  }

  @Test
  void convertOptionsBuilderRejectsNegativeRecursionLimit() {
    assertThrows(IllegalArgumentException.class, () -> {
      ConvertOptions.builder().recursionLimit(-5).build();
    });
  }

  // ---------------------------------------------------------------
  // Round-trip integration tests
  // ---------------------------------------------------------------

  @Test
  void roundTripWithTypedApi() throws Exception {
    JsonNode schema = MAPPER.readTree("""
        {
          "type": "object",
          "properties": {
            "name": { "type": "string" },
            "score": { "type": "integer", "minimum": 0, "maximum": 100 }
          },
          "required": ["name", "score"]
        }
        """);

    ConvertResult convertResult = engine.convertTyped(schema);

    JsonNode llmData = MAPPER.readTree("{\"name\": \"Ada\", \"score\": 95}");
    RehydrateResult rehydrateResult = engine.rehydrateTyped(
        llmData, convertResult.codec(), schema);

    assertEquals("Ada", rehydrateResult.data().get("name").asText());
    assertEquals(95, rehydrateResult.data().get("score").asInt());
  }

  // ---------------------------------------------------------------
  // Error propagation tests
  // ---------------------------------------------------------------

  @Test
  void convertResultFromJsonThrowsOnNullInput() {
    // The fromJson factory should throw on null input
    assertThrows(IllegalArgumentException.class, () -> {
      ConvertResult.fromJson(null);
    });
  }

  @Test
  void rehydrateResultFromJsonThrowsOnMissingData() {
    ObjectNode raw = MAPPER.createObjectNode();
    raw.put("apiVersion", "1.0");
    // Missing "data" field
    assertThrows(IllegalArgumentException.class, () -> {
      RehydrateResult.fromJson(raw);
    });
  }
}
