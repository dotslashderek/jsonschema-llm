package com.jsonschema.llm.wasi;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;

/**
 * Typed result of a {@code jsl_rehydrate} WASI call.
 *
 * <p>Wraps the raw JSON payload returned by the FFI into a strongly-typed
 * record, eliminating the need for consumers to manually extract fields.
 *
 * <p>The expected JSON shape is:
 * <pre>{@code
 * {
 * "apiVersion": "1.0",
 * "data": { ... },
 * "warnings": [ ... ]
 * }
 * }</pre>
 *
 * @param apiVersion FFI envelope version (e.g. "1.0")
 * @param data the rehydrated data restored to match the original schema shape
 * @param warnings advisory warnings about constraint violations or type
 * coercions
 */
public record RehydrateResult(
        String apiVersion,
        JsonNode data,
        JsonNode warnings) {

    /**
     * Deserializes a {@code RehydrateResult} from the raw JSON payload returned
     * by the WASI {@code jsl_rehydrate} export.
     *
     * @param raw the raw JSON node from the FFI call
     * @return a typed {@code RehydrateResult}
     * @throws IllegalArgumentException if required fields are missing
     */
    public static RehydrateResult fromJson(JsonNode raw) {
        if (raw == null) {
            throw new IllegalArgumentException("raw JsonNode must not be null");
        }
        JsonNode dataNode = raw.get("data");
        if (dataNode == null) {
            throw new IllegalArgumentException(
                    "RehydrateResult JSON must contain 'data' field, got JSON: "
                            + raw.toString());
        }

        String apiVersion = raw.has("apiVersion") ? raw.get("apiVersion").asText() : null;
        JsonNode warnings = raw.has("warnings") ? raw.get("warnings")
                : JsonNodeFactory.instance.arrayNode();

        return new RehydrateResult(apiVersion, dataNode, warnings);
    }
}
