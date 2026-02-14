package com.jsonschema.llm;

import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.io.IOException;

public class JsonSchemaLlm {
    private static final Binding BINDING;
    private static final ObjectMapper MAPPER = new ObjectMapper()
        .configure(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false);

    static {
        boolean forceJni = Boolean.getBoolean("com.jsonschema.llm.forceJni");
        Binding binding = null;

        if (!forceJni) {
            try {
                // Try to load PanamaBinding (Java 22+)
                Class<?> panamaClass = Class.forName("com.jsonschema.llm.PanamaBinding");
                binding = (Binding) panamaClass.getDeclaredConstructor().newInstance();
            } catch (Throwable t) {
                // Fallback or ignore
                // System.err.println("Panama binding not available: " + t);
            }
        }

        if (binding == null) {
            binding = new JniBinding();
        }

        BINDING = binding;
    }

    public static ConvertResult convert(JsonNode schema, ConvertOptions options) {
        try {
            String schemaJson = MAPPER.writeValueAsString(schema);
            String optionsJson = MAPPER.writeValueAsString(options);

            String resultJson = BINDING.convertJson(schemaJson, optionsJson);
            if (resultJson == null) {
                 throw new RuntimeException("Binding returned null result");
            }

            JsonNode root = MAPPER.readTree(resultJson);

            if (root.has("apiVersion")) {
                return MAPPER.treeToValue(root, ConvertResult.class);
            } else {
                throw parseError(root);
            }
        } catch (IOException e) {
            throw new RuntimeException("JSON serialization error", e);
        }
    }

    public static RehydrateResult rehydrate(JsonNode data, JsonNode codec, JsonNode originalSchema) {
        try {
            String dataJson = MAPPER.writeValueAsString(data);
            String codecJson = MAPPER.writeValueAsString(codec);
            String schemaJson = MAPPER.writeValueAsString(originalSchema);

            String resultJson = BINDING.rehydrateJson(dataJson, codecJson, schemaJson);
            if (resultJson == null) {
                 throw new RuntimeException("Binding returned null result");
            }

            JsonNode root = MAPPER.readTree(resultJson);

            if (root.has("apiVersion")) {
                return MAPPER.treeToValue(root, RehydrateResult.class);
            } else {
                throw parseError(root);
            }
        } catch (IOException e) {
            throw new RuntimeException("JSON serialization error", e);
        }
    }

    private static JsonSchemaLlmException parseError(JsonNode root) {
        String code = root.has("code") ? root.get("code").asText() : "UnknownError";
        String message = root.has("message") ? root.get("message").asText() : "Unknown error";
        String path = root.has("path") ? root.get("path").asText() : null;
        return new JsonSchemaLlmException(message, code, path);
    }
}
