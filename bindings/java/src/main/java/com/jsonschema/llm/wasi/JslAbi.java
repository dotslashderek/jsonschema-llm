package com.jsonschema.llm.wasi;

import com.dylibso.chicory.runtime.ExportFunction;
import com.dylibso.chicory.runtime.Instance;
import com.dylibso.chicory.runtime.Memory;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/**
 * Package-private ABI marshalling helper for the jsonschema-llm WASM module.
 *
 * <p>
 * Encapsulates the memory protocol (allocation, result struct parsing,
 * cleanup) shared between {@link SchemaLlmEngine} and potentially other
 * consumers. Extracted from {@link JsonSchemaLlmWasi#callJsl} to avoid
 * duplication.
 */
final class JslAbi {

    static final int JSL_RESULT_SIZE = 12; // 3 × u32 (LE)
    static final int STATUS_OK = 0;
    static final int STATUS_ERROR = 1;
    static final int EXPECTED_ABI_VERSION = 1;

    private static final ObjectMapper MAPPER = new ObjectMapper();

    private JslAbi() {
        // Utility class — no instances
    }

    /**
     * Verify the ABI version of the WASM module.
     *
     * @param instance the WASM instance to check
     * @throws RuntimeException if the version is incompatible or missing
     */
    static void verifyAbi(Instance instance) {
        ExportFunction abiFn = instance.export("jsl_abi_version");
        if (abiFn == null) {
            throw new RuntimeException(
                    "Incompatible WASM module: missing required 'jsl_abi_version' export");
        }
        long[] abiResult = abiFn.apply();
        int version = (int) abiResult[0];
        if (version != EXPECTED_ABI_VERSION) {
            throw new RuntimeException(
                    "ABI version mismatch: binary=" + version + ", expected=" + EXPECTED_ABI_VERSION);
        }
    }

    /**
     * Call a WASM export function with the JSL ABI protocol.
     *
     * <p>
     * Handles: argument allocation → function call → result struct parsing
     * → memory cleanup.
     *
     * @param instance the WASM instance (fresh per call)
     * @param funcName the export function name (e.g. "jsl_convert")
     * @param jsonArgs JSON string arguments to pass
     * @return the parsed JSON payload from the result
     * @throws JslException if the WASM module returns an error
     *                      status
     */
    static JsonNode callExport(Instance instance, String funcName, String... jsonArgs)
            throws JslException {
        Memory memory = instance.memory();
        ExportFunction jslAlloc = instance.export("jsl_alloc");
        ExportFunction jslFree = instance.export("jsl_free");
        ExportFunction jslResultFree = instance.export("jsl_result_free");
        ExportFunction func = instance.export(funcName);

        List<int[]> allocs = new ArrayList<>();
        List<Long> flatArgs = new ArrayList<>();
        int resultPtr = 0;

        try {
            for (String arg : jsonArgs) {
                byte[] bytes = arg.getBytes(StandardCharsets.UTF_8);
                long[] allocResult = jslAlloc.apply(bytes.length);
                int ptr = (int) allocResult[0];
                if (ptr == 0 && bytes.length > 0) {
                    throw new RuntimeException("jsl_alloc returned null for " + bytes.length + " bytes");
                }
                memory.write(ptr, bytes);
                allocs.add(new int[] { ptr, bytes.length });
                flatArgs.add((long) ptr);
                flatArgs.add((long) bytes.length);
            }

            // Call the export function
            long[] wasmArgs = flatArgs.stream().mapToLong(Long::longValue).toArray();
            long[] result = func.apply(wasmArgs);
            resultPtr = (int) result[0];
            if (resultPtr == 0) {
                throw new RuntimeException(funcName + " returned null result pointer");
            }

            // Read JslResult struct (12 bytes: 3 × LE u32)
            byte[] resultBytes = memory.readBytes(resultPtr, JSL_RESULT_SIZE);
            ByteBuffer buf = ByteBuffer.wrap(resultBytes).order(ByteOrder.LITTLE_ENDIAN);
            int status = buf.getInt();
            int payloadPtr = buf.getInt();
            int payloadLen = buf.getInt();

            if (payloadPtr < 0 || payloadLen < 0) {
                throw new RuntimeException(
                        "invalid payload pointer/length: ptr=" + payloadPtr + " len=" + payloadLen);
            }

            byte[] payloadBytes = memory.readBytes(payloadPtr, payloadLen);
            String payloadStr = new String(payloadBytes, StandardCharsets.UTF_8);
            JsonNode payload = MAPPER.readTree(payloadStr);

            if (status == STATUS_ERROR) {
                throw new JslException(
                        payload.path("code").asText("unknown"),
                        payload.path("message").asText("unknown error"),
                        payload.path("path").asText(""));
            }

            if (status != STATUS_OK) {
                throw new RuntimeException(
                        "ABI protocol violation: unexpected status=" + status
                                + " (expected 0=OK or 1=ERROR). Payload: "
                                + payloadStr.substring(0, Math.min(200, payloadStr.length())));
            }

            return payload;
        } catch (JslException e) {
            throw e;
        } catch (Exception e) {
            throw new RuntimeException("callExport failed: " + funcName, e);
        } finally {
            if (resultPtr != 0) {
                jslResultFree.apply(resultPtr);
            }
            for (int[] alloc : allocs) {
                jslFree.apply(alloc[0], alloc[1]);
            }
        }
    }
}
