package com.jsonschema.llm.wasi;

import com.fasterxml.jackson.databind.JsonNode;

/**
 * Typed result of a {@code jsl_convert} WASI call.
 *
 * <p>Wraps the raw JSON payload returned by the FFI into a strongly-typed
 * record, eliminating the need for consumers to manually extract fields.
 *
 * <p>The expected JSON shape is:
 * <pre>{@code
 * {
 * "apiVersion": "1.0",
 * "schema": { ... },
 * "codec": { ... },
 * "providerCompatErrors": [ ... ] // optional
 * }
 * }</pre>
 *
 * @param apiVersion FFI envelope version (e.g. "1.0")
 * @param schema the converted LLM-compatible schema
 * @param codec the codec sidecar needed for rehydration
 * @param providerCompatErrors optional provider compatibility warnings (may be
 * null)
 */
public record ConvertResult(
        String apiVersion,
        JsonNode schema,
        JsonNode codec,
        JsonNode providerCompatErrors) {

    /**
     * Deserializes a {@code ConvertResult} from the raw JSON payload returned
     * by the WASI {@code jsl_convert} export.
     *
     * @param raw the raw JSON node from the FFI call
     * @return a typed {@code ConvertResult}
     * @throws IllegalArgumentException if required fields are missing
     */
    public static ConvertResult fromJson(JsonNode raw) {
        if (raw == null) {
            throw new IllegalArgumentException("raw JsonNode must not be null");
        }
        JsonNode schemaNode = raw.get("schema");
        JsonNode codecNode = raw.get("codec");
        if (schemaNode == null || codecNode == null) {
            throw new IllegalArgumentException(
                    "ConvertResult JSON must contain 'schema' and 'codec' fields, got JSON: "
                            + raw.toString());
        }

        String apiVersion = raw.has("apiVersion") ? raw.get("apiVersion").asText() : null;
        JsonNode providerCompatErrors = raw.get("providerCompatErrors");
        // Treat explicit JSON null as absent
        if (providerCompatErrors != null && providerCompatErrors.isNull()) {
            providerCompatErrors = null;
        }

        return new ConvertResult(apiVersion, schemaNode, codecNode, providerCompatErrors);
    }
}
