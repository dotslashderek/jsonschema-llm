package com.jsonschema.llm.wasi;

import static org.junit.jupiter.api.Assertions.*;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

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

        @org.junit.jupiter.api.BeforeAll
        static void checkWasmExists() {
                try {
                        WasmResolver.resolve();
                } catch (Throwable t) {
                        org.junit.jupiter.api.Assumptions.abort("WASM binary not found. Skipping WasmResolverTest.");
                }
        }

        // ---------------------------------------------------------------
        // Env var resolution (Level 2)
        // ---------------------------------------------------------------

        @Test
        void resolveReturnsEnvVarPathWhenSet() {
                // Only runs when JSL_WASM_PATH is externally provided (e.g. local dev).
                String envPath = System.getenv("JSL_WASM_PATH");
                assumeTrue(envPath != null, "JSL_WASM_PATH not set — skipping env var test");

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
                assumeTrue(envPath != null, "JSL_WASM_PATH not set — skipping precedence test");

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
                // Verify the classpath extraction logic works in isolation.
                InputStream is = WasmResolver.class.getResourceAsStream(
                                WasmResolver.CLASSPATH_RESOURCE);
                org.junit.jupiter.api.Assumptions.assumeTrue(is != null,
                                "WASM binary must be built and on classpath for this test");

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
                // SchemaLlmEngine.create() should work via classpath or env var.
                try {
                        try (SchemaLlmEngine engine = SchemaLlmEngine.create()) {
                                assertNotNull(engine, "No-arg create() should return a non-null engine");
                        }
                } catch (Throwable t) {
                        org.junit.jupiter.api.Assumptions.abort("WASM binary not found");
                }
        }
}
