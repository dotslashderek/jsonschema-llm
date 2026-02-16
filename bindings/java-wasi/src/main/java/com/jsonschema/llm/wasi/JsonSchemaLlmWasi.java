package com.jsonschema.llm.wasi;

import com.dylibso.chicory.runtime.Instance;
import com.dylibso.chicory.runtime.Memory;
import com.dylibso.chicory.runtime.Module;
import com.dylibso.chicory.wasi.WasiOptions;
import com.dylibso.chicory.wasi.WasiPreview1;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;

import java.io.File;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/**
 * WASI-backed wrapper for jsonschema-llm.
 *
 * <p>
 * Uses Chicory (pure Java, zero native deps) to load the universal WASI binary.
 *
 * <p>
 * Concurrency: Each Engine owns its own Module. NOT thread-safe.
 */
public class JsonSchemaLlmWasi implements AutoCloseable {

    private static final int JSL_RESULT_SIZE = 12; // 3 × u32 (LE)
    private static final int STATUS_OK = 0;
    private static final int STATUS_ERROR = 1;
    private static final ObjectMapper MAPPER = new ObjectMapper();

    private final Module module;

    public JsonSchemaLlmWasi() {
        this(System.getenv("JSL_WASM_PATH") != null
                ? System.getenv("JSL_WASM_PATH")
                : "target/wasm32-wasip1/release/jsonschema_llm_wasi.wasm");
    }

    public JsonSchemaLlmWasi(String wasmPath) {
        this.module = Module.builder(new File(wasmPath))
                .withHostImports(
                        WasiPreview1.builder().withOptions(WasiOptions.builder().build()).build().toHostImports())
                .build();
    }

    @Override
    public void close() {
        // Module doesn't need explicit cleanup in Chicory
    }

    public JsonNode convert(Object schema) throws JslException {
        return convert(schema, null);
    }

    public JsonNode convert(Object schema, Object options) throws JslException {
        try {
            String schemaJson = MAPPER.writeValueAsString(schema);
            String optsJson = MAPPER.writeValueAsString(options != null ? options : new Object() {
            });
            return callJsl("jsl_convert", schemaJson, optsJson);
        } catch (JslException e) {
            throw e;
        } catch (Exception e) {
            throw new RuntimeException("convert failed", e);
        }
    }

    public JsonNode rehydrate(Object data, Object codec, Object schema) throws JslException {
        try {
            String dataJson = MAPPER.writeValueAsString(data);
            String codecJson = MAPPER.writeValueAsString(codec);
            String schemaJson = MAPPER.writeValueAsString(schema);
            return callJsl("jsl_rehydrate", dataJson, codecJson, schemaJson);
        } catch (JslException e) {
            throw e;
        } catch (Exception e) {
            throw new RuntimeException("rehydrate failed", e);
        }
    }

    JsonNode callJsl(String funcName, String... jsonArgs) throws JslException {
        try {
            // Fresh instance per call
            Instance instance = module.instantiate();
            Memory memory = instance.memory();

            // Allocate and write arguments
            List<int[]> allocs = new ArrayList<>();
            List<Long> flatArgs = new ArrayList<>();

            for (String arg : jsonArgs) {
                byte[] bytes = arg.getBytes(StandardCharsets.UTF_8);
                long[] allocResult = instance.export("jsl_alloc").apply(bytes.length);
                int ptr = (int) allocResult[0];
                memory.write(ptr, bytes);
                allocs.add(new int[] { ptr, bytes.length });
                flatArgs.add((long) ptr);
                flatArgs.add((long) bytes.length);
            }

            // Call function
            long[] result = instance.export(funcName).apply(
                    flatArgs.stream().mapToLong(Long::longValue).toArray());
            int resultPtr = (int) result[0];

            // Read JslResult (12 bytes: 3 × LE u32)
            byte[] resultBytes = memory.readBytes(resultPtr, JSL_RESULT_SIZE);
            ByteBuffer buf = ByteBuffer.wrap(resultBytes).order(ByteOrder.LITTLE_ENDIAN);
            int status = buf.getInt();
            int payloadPtr = buf.getInt();
            int payloadLen = buf.getInt();

            // Read payload
            byte[] payloadBytes = memory.readBytes(payloadPtr, payloadLen);
            String payloadStr = new String(payloadBytes, StandardCharsets.UTF_8);

            // Free
            instance.export("jsl_result_free").apply(resultPtr);
            for (int[] alloc : allocs) {
                instance.export("jsl_free").apply(alloc[0], alloc[1]);
            }

            JsonNode payload = MAPPER.readTree(payloadStr);

            if (status == STATUS_ERROR) {
                throw new JslException(
                        payload.path("code").asText("unknown"),
                        payload.path("message").asText("unknown error"),
                        payload.path("path").asText(""));
            }

            return payload;
        } catch (JslException e) {
            throw e;
        } catch (Exception e) {
            throw new RuntimeException("callJsl failed: " + funcName, e);
        }
    }

    public static class JslException extends Exception {
        private final String code;
        private final String path;

        public JslException(String code, String message, String path) {
            super(String.format("jsl error [%s]%s: %s",
                    code,
                    path != null && !path.isEmpty() ? " at " + path : "",
                    message));
            this.code = code;
            this.path = path;
        }

        public String getCode() {
            return code;
        }

        public String getPath() {
            return path;
        }
    }
}
