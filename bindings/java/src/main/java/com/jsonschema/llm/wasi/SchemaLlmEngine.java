package com.jsonschema.llm.wasi;

import com.dylibso.chicory.log.SystemLogger;
import com.dylibso.chicory.runtime.HostImports;
import com.dylibso.chicory.runtime.Instance;
import com.dylibso.chicory.runtime.Module;
import com.dylibso.chicory.wasi.WasiPreview1;
import com.fasterxml.jackson.databind.ObjectMapper;

import java.io.File;
import java.nio.file.Path;

/**
 * High-level facade for jsonschema-llm WASM operations.
 *
 * <p>
 * Compiles the WASM {@link Module} once at construction time and creates
 * a fresh {@link Instance} for each operation call. Each Instance owns its
 * own linear memory, making the engine <strong>thread-safe</strong> for
 * concurrent {@code convert()} and {@code rehydrate()} calls.
 *
 * <p>
 * Implements {@link AutoCloseable} to signal that the engine holds a
 * cached module resource. After {@link #close()}, all operations throw
 * {@link IllegalStateException}.
 *
 * <p>
 * Usage:
 * 
 * <pre>{@code
 * try (SchemaLlmEngine engine = SchemaLlmEngine.create(wasmPath)) {
 *     ConvertResult result = engine.convert(schema);
 *     RehydrateResult rehydrated = engine.rehydrate(data, result.codec(), schema);
 * }
 * }</pre>
 *
 * @see ConvertResult
 * @see RehydrateResult
 * @see ConvertOptions
 */
public class SchemaLlmEngine implements AutoCloseable {

    private static final ObjectMapper MAPPER = new ObjectMapper();

    private final Module cachedModule;
    private final WasiPreview1 wasi;
    private volatile boolean closed = false;
    private volatile boolean abiVerified = false;

    /**
     * Private constructor — use {@link #create(Path)} factory.
     */
    private SchemaLlmEngine(Path wasmPath) {
        if (wasmPath == null) {
            throw new IllegalArgumentException("wasmPath must not be null");
        }
        File wasmFile = wasmPath.toFile();
        if (!wasmFile.exists()) {
            throw new IllegalArgumentException("WASM file not found: " + wasmPath);
        }

        // Build Module once — this is the expensive step (~50-100ms).
        // In Chicory 0.0.12, HostImports (WASI) are coupled to Module at build time.
        // Our WASM module is pure-compute (no I/O), so sharing the WASI context is
        // safe.
        // IMPORTANT: WasiPreview1 must stay alive for the engine's lifetime because
        // the Module's host functions reference it.
        this.wasi = new WasiPreview1(new SystemLogger());
        HostImports hostImports = new HostImports(wasi.toHostFunctions());
        this.cachedModule = Module.builder(wasmFile)
                .withHostImports(hostImports)
                .build();
    }

    /**
     * Create an engine for the given WASM binary path.
     *
     * <p>
     * The WASM binary is compiled once during this call. Subsequent
     * {@code convert()} and {@code rehydrate()} calls reuse the compiled
     * module.
     *
     * @param wasmPath path to the jsonschema-llm WASM binary
     * @return a new engine instance
     * @throws IllegalArgumentException if the WASM file does not exist
     */
    public static SchemaLlmEngine create(Path wasmPath) {
        return new SchemaLlmEngine(wasmPath);
    }

    /**
     * Create an engine using automatic WASM binary discovery.
     *
     * <p>
     * Resolution cascade:
     * <ol>
     * <li>System property {@code jsl.wasm.path}</li>
     * <li>Environment variable {@code JSL_WASM_PATH}</li>
     * <li>Classpath resource {@code /wasm/jsonschema_llm_wasi.wasm}</li>
     * </ol>
     *
     * @return a new engine instance
     * @throws WasmNotFoundException if the WASM binary cannot be found
     */
    public static SchemaLlmEngine create() {
        return new SchemaLlmEngine(WasmResolver.defaultPath());
    }

    /**
     * Convert a JSON Schema to an LLM-optimized format with default options.
     *
     * <p>
     * Thread-safe: creates a fresh WASM Instance per call.
     *
     * @param schema the JSON Schema (any Jackson-serializable object)
     * @return a typed {@link ConvertResult} with schema, codec, and metadata
     * @throws JsonSchemaLlmWasi.JslException if the WASM module returns an error
     * @throws IllegalStateException          if the engine has been closed
     */
    public ConvertResult convert(Object schema) throws JsonSchemaLlmWasi.JslException {
        return convert(schema, null);
    }

    /**
     * Convert a JSON Schema to an LLM-optimized format with specific options.
     *
     * <p>
     * Thread-safe: creates a fresh WASM Instance per call.
     *
     * @param schema  the JSON Schema (any Jackson-serializable object)
     * @param options conversion options built via {@link ConvertOptions#builder()},
     *                or null for defaults
     * @return a typed {@link ConvertResult} with schema, codec, and metadata
     * @throws JsonSchemaLlmWasi.JslException if the WASM module returns an error
     * @throws IllegalStateException          if the engine has been closed
     */
    public ConvertResult convert(Object schema, ConvertOptions options)
            throws JsonSchemaLlmWasi.JslException {
        ensureOpen();
        try {
            String schemaJson = MAPPER.writeValueAsString(schema);
            String optsJson = options != null ? options.toJson() : "{}";

            Instance instance = cachedModule.instantiate();
            verifyAbiOnce(instance);
            com.fasterxml.jackson.databind.JsonNode raw = JslAbi.callExport(instance, "jsl_convert", schemaJson,
                    optsJson);
            return ConvertResult.fromJson(raw);
        } catch (JsonSchemaLlmWasi.JslException e) {
            throw e;
        } catch (Exception e) {
            throw new RuntimeException("convert failed", e);
        }
    }

    /**
     * Rehydrate LLM output back to the original schema structure.
     *
     * <p>
     * Thread-safe: creates a fresh WASM Instance per call.
     *
     * @param data   the LLM-generated JSON data
     * @param codec  the codec sidecar from a prior conversion
     * @param schema the original JSON Schema
     * @return a typed {@link RehydrateResult} with data and warnings
     * @throws JsonSchemaLlmWasi.JslException if the WASM module returns an error
     * @throws IllegalStateException          if the engine has been closed
     */
    public RehydrateResult rehydrate(Object data, Object codec, Object schema)
            throws JsonSchemaLlmWasi.JslException {
        ensureOpen();
        try {
            String dataJson = MAPPER.writeValueAsString(data);
            String codecJson = MAPPER.writeValueAsString(codec);
            String schemaJson = MAPPER.writeValueAsString(schema);

            Instance instance = cachedModule.instantiate();
            verifyAbiOnce(instance);
            com.fasterxml.jackson.databind.JsonNode raw = JslAbi.callExport(instance, "jsl_rehydrate", dataJson,
                    codecJson, schemaJson);
            return RehydrateResult.fromJson(raw);
        } catch (JsonSchemaLlmWasi.JslException e) {
            throw e;
        } catch (Exception e) {
            throw new RuntimeException("rehydrate failed", e);
        }
    }

    /**
     * Release the cached Module. After this call, all operations throw
     * {@link IllegalStateException}.
     *
     * <p>
     * Idempotent — safe to call multiple times.
     */
    @Override
    public void close() {
        closed = true;
        try {
            wasi.close();
        } catch (Exception e) {
            // Best-effort cleanup
        }
    }

    private void ensureOpen() {
        if (closed) {
            throw new IllegalStateException("SchemaLlmEngine has been closed");
        }
    }

    private void verifyAbiOnce(Instance instance) {
        if (!abiVerified) {
            synchronized (this) {
                if (!abiVerified) {
                    JslAbi.verifyAbi(instance);
                    abiVerified = true;
                }
            }
        }
    }
}
