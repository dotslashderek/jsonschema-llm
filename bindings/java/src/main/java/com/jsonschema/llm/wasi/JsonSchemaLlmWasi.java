package com.jsonschema.llm.wasi;

import com.dylibso.chicory.log.SystemLogger;
import com.dylibso.chicory.runtime.ExportFunction;
import com.dylibso.chicory.runtime.HostImports;
import com.dylibso.chicory.runtime.Instance;
import com.dylibso.chicory.runtime.Memory;
import com.dylibso.chicory.runtime.Module;
import com.dylibso.chicory.wasi.WasiPreview1;
import com.dylibso.chicory.wasm.types.Value;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;

import java.io.File;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/**
 * WASI-backed wrapper for jsonschema-llm.
 *
 * <p>
 * Uses Chicory (pure Java, zero native deps) to load the universal WASI binary.
 *
 * <p>
 * Concurrency: Builds a new Module/Instance per call. NOT thread-safe; use one
 * JsonSchemaLlmWasi per thread or synchronize externally.
 */
public class JsonSchemaLlmWasi implements AutoCloseable {

    private static final int JSL_RESULT_SIZE = 12; // 3 × u32 (LE)
    private static final int STATUS_OK = 0;
    private static final int STATUS_ERROR = 1;
    private static final int EXPECTED_ABI_VERSION = 1;
    private static final ObjectMapper MAPPER = new ObjectMapper();

    private final File wasmFile;
    private boolean abiVerified = false;

    public JsonSchemaLlmWasi() {
        this(System.getenv("JSL_WASM_PATH") != null
                ? System.getenv("JSL_WASM_PATH")
                : "target/wasm32-wasip1/release/jsonschema_llm_wasi.wasm");
    }

    public JsonSchemaLlmWasi(String wasmPath) {
        this.wasmFile = new File(wasmPath);
    }

    @Override
    public void close() {
        // No resources to close
    }

    public JsonNode convert(Object schema) throws JslException {
        return convert(schema, null);
    }

    private String normalizeOptionsJson(Object options) throws com.fasterxml.jackson.core.JsonProcessingException {
        if (options == null)
            return "{}";
        // Serialize to JsonNode, then recursively normalize keys
        JsonNode node = MAPPER.valueToTree(options);
        if (node.isObject()) {
            node = normalizeKeys(node);
        }
        return MAPPER.writeValueAsString(node);
    }

    private JsonNode normalizeKeys(JsonNode node) {
        if (!node.isObject())
            return node;
        com.fasterxml.jackson.databind.node.ObjectNode result = MAPPER.createObjectNode();
        node.fields().forEachRemaining(entry -> {
            // Convert camelCase/snake_case to kebab-case
            String key = entry.getKey()
                    .replaceAll("([a-z])([A-Z])", "$1-$2")
                    .replace('_', '-')
                    .toLowerCase();
            result.set(key, normalizeKeys(entry.getValue()));
        });
        return result;
    }

    public JsonNode convert(Object schema, Object options) throws JslException {
        try {
            String schemaJson = MAPPER.writeValueAsString(schema);
            String optsJson = normalizeOptionsJson(options);
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
        // Fresh WASI + instance per call (WASI modules are single-use)
        try (WasiPreview1 wasi = new WasiPreview1(new SystemLogger())) {
            HostImports hostImports = new HostImports(wasi.toHostFunctions());
            Module module = Module.builder(wasmFile)
                    .withHostImports(hostImports)
                    .build();
            Instance instance = module.instantiate();
            Memory memory = instance.memory();

            ExportFunction jslAlloc = instance.export("jsl_alloc");
            ExportFunction jslFree = instance.export("jsl_free");
            ExportFunction jslResultFree = instance.export("jsl_result_free");
            ExportFunction func = instance.export(funcName);

            // ABI version handshake (once per engine lifetime)
            if (!abiVerified) {
                try {
                    ExportFunction abiFn = instance.export("jsl_abi_version");
                    if (abiFn == null) {
                        throw new RuntimeException(
                                "Incompatible WASM module: missing required 'jsl_abi_version' export");
                    }
                    Value[] abiResult = abiFn.apply();
                    int version = abiResult[0].asInt();
                    if (version != EXPECTED_ABI_VERSION) {
                        throw new RuntimeException(
                                "ABI version mismatch: binary=" + version + ", expected=" + EXPECTED_ABI_VERSION);
                    }
                } catch (RuntimeException e) {
                    throw e;
                } catch (Exception e) {
                    throw new RuntimeException("ABI handshake failed", e);
                }
                abiVerified = true;
            }

            // Allocate and write arguments
            List<int[]> allocs = new ArrayList<>();
            List<Value> flatArgs = new ArrayList<>();
            int resultPtr = 0;

            try {
                for (String arg : jsonArgs) {
                    byte[] bytes = arg.getBytes(StandardCharsets.UTF_8);
                    Value[] allocResult = jslAlloc.apply(Value.i32(bytes.length));
                    int ptr = allocResult[0].asInt();
                    if (ptr == 0 && bytes.length > 0) {
                        throw new RuntimeException("jsl_alloc returned null for " + bytes.length + " bytes");
                    }
                    memory.write(ptr, bytes);
                    allocs.add(new int[] { ptr, bytes.length });
                    flatArgs.add(Value.i32(ptr));
                    flatArgs.add(Value.i32(bytes.length));
                }

                // Call function
                Value[] result = func.apply(flatArgs.toArray(new Value[0]));
                resultPtr = result[0].asInt();
                if (resultPtr == 0) {
                    throw new RuntimeException(funcName + " returned null result pointer");
                }

                // Read JslResult (12 bytes: 3 × LE u32)
                byte[] resultBytes = memory.readBytes(resultPtr, JSL_RESULT_SIZE);
                ByteBuffer buf = ByteBuffer.wrap(resultBytes).order(ByteOrder.LITTLE_ENDIAN);
                int status = buf.getInt();
                int payloadPtr = buf.getInt();
                int payloadLen = buf.getInt();

                // Validate payload bounds
                if (payloadPtr < 0 || payloadLen < 0) {
                    throw new RuntimeException(
                            "invalid payload pointer/length: ptr=" + payloadPtr + " len=" + payloadLen);
                }

                // Read payload
                byte[] payloadBytes = memory.readBytes(payloadPtr, payloadLen);
                String payloadStr = new String(payloadBytes, StandardCharsets.UTF_8);

                JsonNode payload = MAPPER.readTree(payloadStr);

                if (status == STATUS_ERROR) {
                    throw new JslException(
                            payload.path("code").asText("unknown"),
                            payload.path("message").asText("unknown error"),
                            payload.path("path").asText(""));
                }

                return payload;
            } finally {
                // Always free guest memory
                if (resultPtr != 0) {
                    jslResultFree.apply(Value.i32(resultPtr));
                }
                for (int[] alloc : allocs) {
                    jslFree.apply(Value.i32(alloc[0]), Value.i32(alloc[1]));
                }
            }
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
