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
        String wasmPath = System.getenv("JSL_WASM_PATH");
        Path path = wasmPath != null && !wasmPath.isEmpty()
                ? Paths.get(wasmPath)
                : Paths.get("../../target/wasm32-wasip1/release/jsonschema_llm_wasi.wasm");
        engine = SchemaLlmEngine.create(path);
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

        // Warm-up call (first instantiation)
        long startFirst = System.nanoTime();
        engine.convert(schema);
        long firstCallMs = (System.nanoTime() - startFirst) / 1_000_000;

        // Subsequent calls should reuse the cached Module
        long startSecond = System.nanoTime();
        engine.convert(schema);
        long secondCallMs = (System.nanoTime() - startSecond) / 1_000_000;

        // Relative speedup: second call should be meaningfully faster
        // (or at least not slower — Module compilation is the expensive part)
        assertTrue(secondCallMs <= firstCallMs || secondCallMs < 50,
                "Second call (" + secondCallMs + "ms) should not be dramatically slower than first ("
                        + firstCallMs + "ms) — Module should be cached");
    }

    // ---------------------------------------------------------------
    // Lifecycle
    // ---------------------------------------------------------------

    @Test
    void closeReleasesResources() throws Exception {
        String wasmPath = System.getenv("JSL_WASM_PATH");
        Path path = wasmPath != null && !wasmPath.isEmpty()
                ? Paths.get(wasmPath)
                : Paths.get("../../target/wasm32-wasip1/release/jsonschema_llm_wasi.wasm");
        SchemaLlmEngine localEngine = SchemaLlmEngine.create(path);

        // Should work before close
        JsonNode schema = MAPPER.readTree("{\"type\": \"object\"}");
        assertNotNull(localEngine.convert(schema));

        localEngine.close();

        // Should throw after close
        assertThrows(IllegalStateException.class, () -> localEngine.convert(schema));
    }

    @Test
    void closeIsIdempotent() throws Exception {
        String wasmPath = System.getenv("JSL_WASM_PATH");
        Path path = wasmPath != null && !wasmPath.isEmpty()
                ? Paths.get(wasmPath)
                : Paths.get("../../target/wasm32-wasip1/release/jsonschema_llm_wasi.wasm");
        SchemaLlmEngine localEngine = SchemaLlmEngine.create(path);

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
    void convertError() {
        // Invalid JSON should propagate as JslException, not raw Chicory exception
        assertThrows(JsonSchemaLlmWasi.JslException.class,
                () -> engine.convert("NOT VALID JSON"));
    }
}
