package com.jsonschema.llm.wasi;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;

/**
 * Fluent builder for conversion options passed to the WASI {@code jsl_convert}
 * export.
 *
 * <p>
 * Produces kebab-case JSON that the WASM ABI expects. Builder methods use
 * idiomatic Java camelCase names; keys are automatically normalized.
 *
 * <p>
 * Usage:
 * 
 * <pre>{@code
 * ConvertOptions opts = ConvertOptions.builder()
 *         .target("openai-strict")
 *         .maxDepth(50)
 *         .recursionLimit(3)
 *         .polymorphism("any-of")
 *         .build();
 *
 * String json = opts.toJson(); // {"target":"openai-strict","max-depth":50,...}
 * }</pre>
 */
public final class ConvertOptions {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private final ObjectNode json;

    private ConvertOptions(ObjectNode json) {
        this.json = json;
    }

    /**
     * Create a new builder.
     *
     * @return a fresh builder instance
     */
    public static Builder builder() {
        return new Builder();
    }

    /**
     * Serialize this options object to a JSON string suitable for the WASM ABI.
     *
     * @return JSON string with kebab-case keys
     */
    public String toJson() {
        try {
            return MAPPER.writeValueAsString(json);
        } catch (JsonProcessingException e) {
            // Should never happen with ObjectNode
            throw new RuntimeException("Failed to serialize ConvertOptions", e);
        }
    }

    /**
     * Fluent builder for {@link ConvertOptions}.
     */
    public static final class Builder {

        private final ObjectNode root = MAPPER.createObjectNode();

        private Builder() {
        }

        /**
         * Set the LLM provider target format.
         * Values: "openai-strict", "gemini", "claude"
         */
        public Builder target(String target) {
            root.put("target", target);
            return this;
        }

        /**
         * Set the conversion mode.
         * Values: "strict" (default), "permissive"
         */
        public Builder mode(String mode) {
            root.put("mode", mode);
            return this;
        }

        /**
         * Set the maximum traversal depth for ref resolution.
         * Default: 50
         */
        public Builder maxDepth(int maxDepth) {
            if (maxDepth < 0) {
                throw new IllegalArgumentException("maxDepth must be non-negative, got: " + maxDepth);
            }
            root.put("max-depth", maxDepth);
            return this;
        }

        /**
         * Set the maximum number of times a recursive type may be inlined.
         * Default: 3
         */
        public Builder recursionLimit(int recursionLimit) {
            if (recursionLimit < 0) {
                throw new IllegalArgumentException(
                        "recursionLimit must be non-negative, got: " + recursionLimit);
            }
            root.put("recursion-limit", recursionLimit);
            return this;
        }

        /**
         * Set the polymorphism strategy.
         * Values: "any-of" (default), "flatten"
         */
        public Builder polymorphism(String polymorphism) {
            root.put("polymorphism", polymorphism);
            return this;
        }

        /**
         * Set an arbitrary option. The key is automatically normalized
         * from camelCase to kebab-case.
         *
         * @param key   option name (camelCase or kebab-case)
         * @param value option value
         * @return this builder
         */
        public Builder option(String key, Object value) {
            String normalizedKey = normalizeKey(key);
            root.putPOJO(normalizedKey, value);
            return this;
        }

        /**
         * Build the immutable options object.
         *
         * @return a new {@link ConvertOptions}
         */
        public ConvertOptions build() {
            return new ConvertOptions(root.deepCopy());
        }

        private static String normalizeKey(String key) {
            return key.replaceAll("([a-z])([A-Z])", "$1-$2")
                    .replace('_', '-')
                    .toLowerCase();
        }
    }
}
