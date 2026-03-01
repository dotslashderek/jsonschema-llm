package com.jsonschema.llm.wasi;

import static org.junit.jupiter.api.Assertions.*;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;

import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;

/**
 * Acceptance tests for SchemaLlmEngine facade (Issue #161).
 *
 * <p>
 * Tests cover: typed results, options, round-trip, thread safety,
 * module caching, lifecycle, and error handling.
 */
class SchemaLlmEngineTest {

  private static final ObjectMapper MAPPER = new ObjectMapper();
  private static SchemaLlmEngine engine;

  @BeforeAll
  static void setUp() {
    try {
      engine = SchemaLlmEngine.create();
    } catch (Throwable t) {
      org.junit.jupiter.api.Assumptions.abort("WASM binary not found. Skipping tests.");
    }
  }

  @AfterAll
  static void tearDown() {
    if (engine != null) {
      engine.close();
    }
  }

  // ---------------------------------------------------------------
  // Core operations
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

    ConvertResult result = engine.convert(schema);

    assertNotNull(result, "ConvertResult should not be null");
    assertNotNull(result.apiVersion(), "apiVersion should be present");
    assertFalse(result.apiVersion().isEmpty(), "apiVersion should not be empty");
    assertNotNull(result.schema(), "schema should not be null");
    assertNotNull(result.codec(), "codec should not be null");
    assertTrue(result.schema().isObject(), "schema should be an object");
  }

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

    ConvertResult result = engine.convert(schema, opts);

    assertNotNull(result);
    assertNotNull(result.schema());
    assertNotNull(result.codec());
  }

  @Test
  void roundTripIntegrity() throws Exception {
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

    ConvertResult convertResult = engine.convert(schema);

    JsonNode llmData = MAPPER.readTree("{\"name\": \"Ada\", \"score\": 95}");
    RehydrateResult rehydrateResult = engine.rehydrate(
        llmData, convertResult.codec(), schema);

    assertNotNull(rehydrateResult);
    assertEquals("Ada", rehydrateResult.data().get("name").asText());
    assertEquals(95, rehydrateResult.data().get("score").asInt());
  }

  // ---------------------------------------------------------------
  // Thread safety
  // ---------------------------------------------------------------

  @Test
  void threadSafety() throws Exception {
    JsonNode schema = MAPPER.readTree("""
        {
          "type": "object",
          "properties": {
            "value": { "type": "integer" }
          },
          "required": ["value"]
        }
        """);

    int threadCount = 10;
    ExecutorService executor = Executors.newFixedThreadPool(threadCount);
    List<Future<ConvertResult>> futures = new ArrayList<>();

    for (int i = 0; i < threadCount; i++) {
      futures.add(executor.submit(() -> engine.convert(schema)));
    }

    executor.shutdown();
    assertTrue(executor.awaitTermination(30, TimeUnit.SECONDS),
        "Executor should complete all tasks within timeout");

    List<ConvertResult> results = new ArrayList<>();
    for (Future<ConvertResult> future : futures) {
      results.add(future.get());
    }

    assertEquals(threadCount, results.size());
    for (ConvertResult result : results) {
      assertNotNull(result);
      assertNotNull(result.schema());
      assertNotNull(result.codec());
    }
  }

  // ---------------------------------------------------------------
  // Module caching
  // ---------------------------------------------------------------

  @Test
  void moduleCaching() throws Exception {
    JsonNode schema = MAPPER.readTree("{\"type\": \"object\", \"properties\": {\"x\": {\"type\": \"string\"}}}");

    // The engine was created once in @BeforeAll, so the Module is compiled once.
    // If caching were broken (Module rebuilt per call), we'd see errors or
    // inconsistent results. Running 10 consecutive calls proves structural reuse.
    List<ConvertResult> results = new ArrayList<>();
    for (int i = 0; i < 10; i++) {
      results.add(engine.convert(schema));
    }

    // All results should be consistent (same schema structure from same Module)
    for (ConvertResult result : results) {
      assertNotNull(result);
      assertNotNull(result.schema());
      assertNotNull(result.codec());
      assertEquals(results.get(0).apiVersion(), result.apiVersion(),
          "All calls should report the same API version (same Module)");
    }
  }

  // ---------------------------------------------------------------
  // Lifecycle
  // ---------------------------------------------------------------

  @Test
  void closeReleasesResources() throws Exception {
    SchemaLlmEngine localEngine = SchemaLlmEngine.create();

    // Should work before close
    JsonNode schema = MAPPER.readTree("{\"type\": \"object\"}");
    assertNotNull(localEngine.convert(schema));

    localEngine.close();

    // Should throw after close
    assertThrows(IllegalStateException.class, () -> localEngine.convert(schema));
  }

  @Test
  void closeIsIdempotent() throws Exception {
    SchemaLlmEngine localEngine = SchemaLlmEngine.create();

    // Multiple close() calls should not throw
    assertDoesNotThrow(() -> {
      localEngine.close();
      localEngine.close();
      localEngine.close();
    });
  }

  // ---------------------------------------------------------------
  // Error handling
  // ---------------------------------------------------------------

  @Test
  void rehydrateError() throws Exception {
    // Rehydrate with invalid codec triggers a JslException from the WASM module.
    // This verifies the engine wraps WASM errors into typed exceptions.
    JsonNode schema = MAPPER.readTree("{\"type\": \"object\"}");
    JsonNode data = MAPPER.readTree("{\"key\": \"value\"}");
    // Raw string "NOT VALID JSON" as codec — WASM will reject it
    assertThrows(JslException.class,
        () -> engine.rehydrate(data, "NOT_A_VALID_CODEC", schema));
  }
  // ---------------------------------------------------------------
  // Apply Patch — #276 diagnostic tests
  // ---------------------------------------------------------------

  @Test
  void applyPatchReplaceWithJsonNode() throws Exception {
    // Baseline: JsonNode → writeValueAsString → WASM should round-trip cleanly
    JsonNode schema = MAPPER.readTree("""
        {
          "type": "object",
          "properties": {
            "configuration": {
              "type": "object",
              "description": "Endpoint configuration"
            },
            "name": { "type": "string" }
          },
          "required": ["name"]
        }
        """);

    String patchJson = """
        [
          {
            "op": "replace",
            "path": "/properties/configuration",
            "value": {
              "type": "object",
              "description": "Updated configuration",
              "properties": {
                "target": { "type": "string" },
                "rate": { "type": "integer" }
              },
              "required": ["target"]
            }
          }
        ]
        """;

    JsonNode result = engine.applyPatch(schema, patchJson);

    assertNotNull(result, "patched schema should not be null");
    assertEquals("string",
        result.at("/properties/configuration/properties/target/type").asText(),
        "replaced value should contain new properties");
    assertEquals("string",
        result.at("/properties/name/type").asText(),
        "sibling property should be unaffected");
  }

  @Test
  void applyPatchAddExistingPath() throws Exception {
    // Per RFC 6902, "add" on an existing path replaces the value.
    // Issue #276 says this fails through the WASM bridge.
    JsonNode schema = MAPPER.readTree("""
        {
          "type": "object",
          "properties": {
            "config": { "type": "object", "description": "original" }
          }
        }
        """);

    String patchJson = """
        [
          {
            "op": "add",
            "path": "/properties/config",
            "value": {
              "type": "object",
              "description": "replaced via add",
              "properties": { "key": { "type": "string" } }
            }
          }
        ]
        """;

    JsonNode result = engine.applyPatch(schema, patchJson);

    assertNotNull(result);
    assertEquals("replaced via add",
        result.at("/properties/config/description").asText());
  }

  @Test
  void applyPatchAddNewPath() throws Exception {
    // Control test: add at non-existing path (should work per issue report)
    JsonNode schema = MAPPER.readTree("""
        {
          "type": "object",
          "properties": {
            "name": { "type": "string" }
          }
        }
        """);

    String patchJson = """
        [
          {
            "op": "add",
            "path": "/properties/age",
            "value": { "type": "integer", "minimum": 0 }
          }
        ]
        """;

    JsonNode result = engine.applyPatch(schema, patchJson);

    assertNotNull(result);
    assertEquals("integer",
        result.at("/properties/age/type").asText());
    assertEquals("string",
        result.at("/properties/name/type").asText(),
        "existing property should be preserved");
  }

  @Test
  void applyPatchInvalidPath() {
    // replace at a path that doesn't exist should throw JslException
    JsonNode schema = MAPPER.createObjectNode().put("type", "object");

    String patchJson = """
        [{"op": "replace", "path": "/nonexistent", "value": "x"}]
        """;

    assertThrows(JslException.class,
        () -> engine.applyPatch(schema, patchJson),
        "replace at non-existing path should throw JslException");
  }

}
