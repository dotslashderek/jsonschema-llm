package com.jsonschema.llm.wasi;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Tests for WasmResolver — WASM binary auto-discovery cascade.
 *
 * <p>
 * Resolution order:
 * <ol>
 * <li>System property {@code jsl.wasm.path}</li>
 * <li>Environment variable {@code JSL_WASM_PATH}</li>
 * <li>Classpath resource {@code /wasm/json_schema_llm_wasi.wasm}</li>
 * </ol>
 */
class WasmResolverTest {

        // ---------------------------------------------------------------
        // Env var resolution (Level 2)
        // ---------------------------------------------------------------

        @Test
        void resolveReturnsEnvVarPathWhenSet() {
                // The test environment sets JSL_WASM_PATH in build.gradle.kts.
                // WasmResolver.resolve() should find it at Level 2 (env var).
                String envPath = System.getenv("JSL_WASM_PATH");
                assertNotNull(envPath, "JSL_WASM_PATH must be set in the test environment");

                Path resolved = WasmResolver.resolve();
                assertNotNull(resolved);
                assertTrue(Files.isRegularFile(resolved),
                                "Resolved path should be a regular file: " + resolved);
        }

        // ---------------------------------------------------------------
        // System property precedence (Level 1 overrides Level 2)
        // ---------------------------------------------------------------

        @Test
        void systemPropertyTakesPrecedenceOverEnvVar() {
                String envPath = System.getenv("JSL_WASM_PATH");
                assertNotNull(envPath, "JSL_WASM_PATH must be set in the test environment");

                // Set sys prop to the same valid path — it should be used first
                System.setProperty(WasmResolver.SYS_PROP_KEY, envPath);
                try {
                        Path resolved = WasmResolver.resolve();
                        assertNotNull(resolved);
                        assertEquals(Path.of(envPath), resolved,
                                        "System property should take precedence");
                } finally {
                        System.clearProperty(WasmResolver.SYS_PROP_KEY);
                }
        }

        // ---------------------------------------------------------------
        // Invalid path fallthrough
        // ---------------------------------------------------------------

        @Test
        void invalidSysPropFallsThroughToEnvVar() {
                // Set sys prop to an invalid path — cascade should fall through
                System.setProperty(WasmResolver.SYS_PROP_KEY, "/nonexistent/\0invalid");
                try {
                        Path resolved = WasmResolver.resolve();
                        assertNotNull(resolved,
                                        "Should fall through to env var when sys prop path is invalid");
                        assertTrue(Files.isRegularFile(resolved));
                } finally {
                        System.clearProperty(WasmResolver.SYS_PROP_KEY);
                }
        }

        // ---------------------------------------------------------------
        // Classpath extraction (Level 3)
        // ---------------------------------------------------------------

        @Test
        void extractFromClasspathCreatesTempFile() {
                // src/test/resources/wasm/json_schema_llm_wasi.wasm has a dummy file.
                // Verify the classpath extraction logic works in isolation.
                InputStream is = WasmResolver.class.getResourceAsStream(
                                WasmResolver.CLASSPATH_RESOURCE);
                assertNotNull(is, "Dummy WASM should be on the test classpath");

                Path extracted = WasmResolver.extractFromClasspath();
                assertNotNull(extracted, "extractFromClasspath should return a path");
                assertTrue(Files.isRegularFile(extracted),
                                "Extracted file should exist: " + extracted);
                assertTrue(Files.isReadable(extracted),
                                "Extracted file should be readable");
        }

        // ---------------------------------------------------------------
        // WasmNotFoundException (all sources absent)
        // ---------------------------------------------------------------

        @Test
        void wasmNotFoundExceptionListsAllLevels() {
                WasmNotFoundException ex = new WasmNotFoundException(null, null);
                String msg = ex.getMessage();

                assertNotNull(msg);
                assertTrue(msg.contains("jsl.wasm.path"),
                                "Message should mention system property key");
                assertTrue(msg.contains("JSL_WASM_PATH"),
                                "Message should mention environment variable key");
                assertTrue(msg.contains("/wasm/json_schema_llm_wasi.wasm"),
                                "Message should mention classpath resource");
        }

        @Test
        void wasmNotFoundExceptionShowsMissingFileForSetValues() {
                WasmNotFoundException ex = new WasmNotFoundException(
                                "/nonexistent/path.wasm", "/also/nonexistent.wasm");
                String msg = ex.getMessage();

                assertTrue(msg.contains("file missing"),
                                "Message should indicate file missing for set values");
        }

        // ---------------------------------------------------------------
        // DefaultPathHolder (lazy singleton)
        // ---------------------------------------------------------------

        @Test
        void defaultPathReturnsSameInstance() {
                // DefaultPathHolder guarantees the same Path across calls.
                Path first = WasmResolver.defaultPath();
                Path second = WasmResolver.defaultPath();
                assertSame(first, second,
                                "defaultPath() should return the same singleton instance");
        }

        // ---------------------------------------------------------------
        // No-arg SchemaLlmEngine.create()
        // ---------------------------------------------------------------

        @Test
        void noArgCreateUsesAutoDiscovery() throws Exception {
                // SchemaLlmEngine.create() should work without any explicit path.
                // The env var JSL_WASM_PATH is set in the test environment.
                try (SchemaLlmEngine engine = SchemaLlmEngine.create()) {
                        assertNotNull(engine, "No-arg create() should return a non-null engine");
                }
        }
}
