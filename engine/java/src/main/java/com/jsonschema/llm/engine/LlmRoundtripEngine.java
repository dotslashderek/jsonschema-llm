package com.jsonschema.llm.engine;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.jsonschema.llm.wasi.ConvertResult;
import com.jsonschema.llm.wasi.JslException;
import com.jsonschema.llm.wasi.RehydrateResult;
import com.jsonschema.llm.wasi.SchemaLlmEngine;
import com.networknt.schema.JsonSchema;
import com.networknt.schema.JsonSchemaFactory;
import com.networknt.schema.SpecVersion;
import com.networknt.schema.ValidationMessage;

import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;

/**
 * Orchestrator for the full LLM roundtrip.
 *
 * <p>
 * Flow:
 * <ol>
 * <li>Convert input schema to LLM-compatible format (via WASI)</li>
 * <li>Format an LLM request using the provider formatter</li>
 * <li>Execute the request via consumer-provided transport</li>
 * <li>Extract the generated content from the response</li>
 * <li>Rehydrate to the original schema shape (via WASI)</li>
 * <li>Validate against the original schema</li>
 * </ol>
 *
 * <p>
 * Thread-safe — the underlying {@link SchemaLlmEngine} creates a fresh
 * WASM Instance per call.
 */
public class LlmRoundtripEngine implements AutoCloseable {

    private static final ObjectMapper MAPPER = new ObjectMapper();

    private final SchemaLlmEngine wasiEngine;

    private LlmRoundtripEngine(SchemaLlmEngine wasiEngine) {
        this.wasiEngine = wasiEngine;
    }

    /**
     * Create an engine with automatic WASM binary discovery.
     *
     * @return a new engine instance
     * @see SchemaLlmEngine#create()
     */
    public static LlmRoundtripEngine create() {
        return new LlmRoundtripEngine(SchemaLlmEngine.create());
    }

    /**
     * Create an engine for a specific WASM binary path.
     *
     * @param wasmPath path to the jsonschema-llm WASM binary
     * @return a new engine instance
     */
    public static LlmRoundtripEngine create(Path wasmPath) {
        return new LlmRoundtripEngine(SchemaLlmEngine.create(wasmPath));
    }

    /**
     * Execute a full roundtrip: convert → format → call LLM → rehydrate → validate.
     *
     * @param schemaJson the original JSON Schema as a string
     * @param prompt     the natural language prompt for the LLM
     * @param formatter  provider-specific request formatter
     * @param config     provider endpoint/model configuration
     * @param transport  consumer-provided HTTP transport
     * @return the roundtrip result with rehydrated data and validation info
     * @throws LlmTransportException if the transport fails
     * @throws EngineException       if schema conversion, rehydration, or parsing
     *                               fails
     */
    public RoundtripResult generate(
            String schemaJson,
            String prompt,
            ProviderFormatter formatter,
            ProviderConfig config,
            LlmTransport transport) throws LlmTransportException {

        JsonNode schema = parseJson(schemaJson, "input schema");

        // 1. Convert schema to LLM-compatible format
        ConvertResult convertResult;
        try {
            convertResult = wasiEngine.convert(schema);
        } catch (JslException e) {
            throw new EngineException.SchemaConversionException(
                    "Schema conversion failed: " + e.getMessage(), e);
        }

        return generateWithConvertResult(schema, convertResult, prompt, formatter, config, transport);
    }

    /**
     * Execute a roundtrip with a pre-converted schema and codec.
     *
     * <p>
     * Skips the convert step — useful when the caller has already
     * converted the schema and wants to reuse the codec.
     *
     * @param schemaJson the original JSON Schema as a string
     * @param codecJson  the codec sidecar JSON from a prior conversion
     * @param prompt     the natural language prompt for the LLM
     * @param llmSchema  the pre-converted LLM-compatible schema
     * @param formatter  provider-specific request formatter
     * @param config     provider endpoint/model configuration
     * @param transport  consumer-provided HTTP transport
     * @return the roundtrip result
     * @throws LlmTransportException if the transport fails
     */
    public RoundtripResult generateWithPreconverted(
            String schemaJson,
            String codecJson,
            JsonNode llmSchema,
            String prompt,
            ProviderFormatter formatter,
            ProviderConfig config,
            LlmTransport transport) throws LlmTransportException {

        JsonNode schema = parseJson(schemaJson, "input schema");
        JsonNode codec = parseJson(codecJson, "codec");

        // 2. Format the LLM request
        LlmRequest request = formatter.format(prompt, llmSchema, config);

        // 3. Execute transport
        String rawResponse = transport.execute(request);

        // 4. Extract content
        String contentJson = formatter.extractContent(rawResponse);
        JsonNode rawLlmResponse = parseJson(rawResponse, "LLM response");

        return rehydrateAndValidate(schema, codec, contentJson, rawLlmResponse);
    }

    @Override
    public void close() {
        wasiEngine.close();
    }

    // ---------------------------------------------------------------
    // Internal
    // ---------------------------------------------------------------

    private RoundtripResult generateWithConvertResult(
            JsonNode schema,
            ConvertResult convertResult,
            String prompt,
            ProviderFormatter formatter,
            ProviderConfig config,
            LlmTransport transport) throws LlmTransportException {

        // 2. Format the LLM request
        LlmRequest request = formatter.format(prompt, convertResult.schema(), config);

        // 3. Execute transport
        String rawResponse = transport.execute(request);

        // 4. Extract content
        String contentJson = formatter.extractContent(rawResponse);
        JsonNode rawLlmResponse = parseJson(rawResponse, "LLM response");

        return rehydrateAndValidate(schema, convertResult.codec(), contentJson, rawLlmResponse);
    }

    private RoundtripResult rehydrateAndValidate(
            JsonNode schema, JsonNode codec, String contentJson, JsonNode rawLlmResponse) {

        JsonNode data = parseJson(contentJson, "LLM content");

        // 5. Rehydrate
        RehydrateResult rehydrateResult;
        try {
            rehydrateResult = wasiEngine.rehydrate(data, codec, schema);
        } catch (JslException e) {
            throw new EngineException.RehydrationException(
                    "Rehydration failed: " + e.getMessage(), e);
        }

        // Extract warnings: use asText() for plain strings so scalar warnings stay
        // clean (no extra quotes), and toString() for structured objects so their
        // content (path, message, coercion rule) is not silently discarded.
        List<String> warnings = new ArrayList<>();
        if (rehydrateResult.warnings() != null && rehydrateResult.warnings().isArray()) {
            for (JsonNode w : rehydrateResult.warnings()) {
                warnings.add(w.isTextual() ? w.asText() : w.toString());
            }
        }

        // 6. Validate against original schema
        List<String> validationErrors = validate(rehydrateResult.data(), schema);

        return new RoundtripResult(
                rehydrateResult.data(),
                rawLlmResponse,
                warnings,
                validationErrors);
    }

    private List<String> validate(JsonNode data, JsonNode schema) {
        try {
            // Auto-detect spec version from $schema keyword, default to Draft-07
            SpecVersion.VersionFlag version = detectSpecVersion(schema);
            JsonSchemaFactory factory = JsonSchemaFactory.getInstance(version);
            JsonSchema jsonSchema = factory.getSchema(schema);

            Set<ValidationMessage> errors = jsonSchema.validate(data);
            return errors.stream()
                    .map(ValidationMessage::getMessage)
                    .toList();
        } catch (Exception e) {
            // If validation setup fails, report as a single validation error
            return List.of("Validation setup failed: " + e.getMessage());
        }
    }

    private SpecVersion.VersionFlag detectSpecVersion(JsonNode schema) {
        if (schema.has("$schema")) {
            String schemaUri = schema.get("$schema").asText();
            if (schemaUri.contains("draft/2020-12")) {
                return SpecVersion.VersionFlag.V202012;
            } else if (schemaUri.contains("draft/2019-09")) {
                return SpecVersion.VersionFlag.V201909;
            } else if (schemaUri.contains("draft-06")) {
                return SpecVersion.VersionFlag.V6;
            } else if (schemaUri.contains("draft-04")) {
                return SpecVersion.VersionFlag.V4;
            }
        }
        return SpecVersion.VersionFlag.V7; // Default to Draft-07
    }

    private static JsonNode parseJson(String json, String context) {
        if (json == null) {
            throw new EngineException.ResponseParsingException(
                    "Cannot parse " + context + ": input is null");
        }
        try {
            return MAPPER.readTree(json);
        } catch (Exception e) {
            throw new EngineException.ResponseParsingException(
                    "Failed to parse " + context + " as JSON: "
                            + json.substring(0, Math.min(200, json.length())),
                    e);
        }
    }
}
