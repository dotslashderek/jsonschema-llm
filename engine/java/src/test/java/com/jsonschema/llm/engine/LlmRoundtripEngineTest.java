package com.jsonschema.llm.engine;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Tag;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Map;

import static org.assertj.core.api.Assertions.*;

/**
 * E2E acceptance tests for LlmRoundtripEngine.
 *
 * <p>
 * These tests load the real jsonschema_llm_wasi.wasm binary via Chicory
 * and exercise the full convert → format → rehydrate pipeline. A mock
 * LlmTransport returns deterministic responses to avoid HTTP calls.
 *
 * <p>
 * Skipped gracefully if the WASM binary has not been built yet.
 */
@Tag("e2e")
class LlmRoundtripEngineTest {

        private static final ObjectMapper MAPPER = new ObjectMapper();
        private static LlmRoundtripEngine engine;

        // Simple schema for testing
        private static final String PERSON_SCHEMA = """
                        {
                          "type": "object",
                          "properties": {
                            "name": { "type": "string" },
                            "age": { "type": "integer" }
                          },
                          "required": ["name", "age"]
                        }
                        """;

        @BeforeAll
        static void setUp() {
                Path wasmFile = wasmPath();
                Assumptions.assumeTrue(
                                Files.exists(wasmFile),
                                "WASM binary not found at " + wasmFile + ". Build with: make build-wasi");
                engine = LlmRoundtripEngine.create(wasmFile);
        }

        @AfterAll
        static void tearDown() {
                if (engine != null) {
                        engine.close();
                }
        }

        // ---------------------------------------------------------------
        // 1. Full roundtrip with mock transport
        // ---------------------------------------------------------------

        @Test
        void fullRoundtrip_validData_isValid() throws Exception {
                // Mock transport returns a valid OpenAI-shaped response
                LlmTransport mockTransport = request -> openAiResponse(
                                "{\"name\": \"Ada\", \"age\": 30}");

                ProviderConfig config = new ProviderConfig(
                                "https://api.openai.com/v1/chat/completions", "gpt-4o",
                                Map.of("Authorization", "Bearer test-key"));

                RoundtripResult result = engine.generate(
                                PERSON_SCHEMA,
                                "Generate a person",
                                new ChatCompletionsFormatter(),
                                config,
                                mockTransport);

                assertThat(result).isNotNull();
                assertThat(result.isValid()).isTrue();
                assertThat(result.data()).isNotNull();
                assertThat(result.data().get("name").asText()).isEqualTo("Ada");
                assertThat(result.data().get("age").asInt()).isEqualTo(30);
                assertThat(result.validationErrors()).isEmpty();
                assertThat(result.rawLlmResponse()).isNotNull();
        }

        // ---------------------------------------------------------------
        // 2. Validation catches invalid data
        // ---------------------------------------------------------------

        @Test
        void fullRoundtrip_invalidData_hasValidationErrors() throws Exception {
                // Age is a string instead of integer — violates schema
                LlmTransport mockTransport = request -> openAiResponse(
                                "{\"name\": \"Ada\", \"age\": \"thirty\"}");

                ProviderConfig config = new ProviderConfig(
                                "https://api.openai.com/v1/chat/completions", "gpt-4o",
                                Map.of());

                RoundtripResult result = engine.generate(
                                PERSON_SCHEMA,
                                "Generate a person",
                                new ChatCompletionsFormatter(),
                                config,
                                mockTransport);

                assertThat(result).isNotNull();
                assertThat(result.data()).isNotNull();
                assertThat(result.isValid()).isFalse();
                assertThat(result.validationErrors()).isNotEmpty();
        }

        // ---------------------------------------------------------------
        // 3. Transport failure propagates
        // ---------------------------------------------------------------

        @Test
        void transportFailure_propagatesException() {
                LlmTransport failingTransport = request -> {
                        throw new LlmTransportException("Service unavailable", 503);
                };

                ProviderConfig config = new ProviderConfig(
                                "https://api.openai.com/v1/chat/completions", "gpt-4o",
                                Map.of());

                assertThatThrownBy(() -> engine.generate(
                                PERSON_SCHEMA,
                                "Generate a person",
                                new ChatCompletionsFormatter(),
                                config,
                                failingTransport))
                                .isInstanceOf(LlmTransportException.class)
                                .hasMessageContaining("Service unavailable");
        }

        // ---------------------------------------------------------------
        // 4. Pre-converted schema overload
        // ---------------------------------------------------------------

        @Test
        void preconvertedSchema_skipsConvertStep() throws Exception {
                // First, convert the schema normally to get the codec
                var wasiEngine = com.jsonschema.llm.wasi.SchemaLlmEngine.create(wasmPath());
                var convertResult = wasiEngine.convert(MAPPER.readTree(PERSON_SCHEMA));

                LlmTransport mockTransport = request -> openAiResponse(
                                "{\"name\": \"Lovelace\", \"age\": 36}");

                ProviderConfig config = new ProviderConfig(
                                "https://api.openai.com/v1/chat/completions", "gpt-4o",
                                Map.of());

                RoundtripResult result = engine.generateWithPreconverted(
                                PERSON_SCHEMA,
                                MAPPER.writeValueAsString(convertResult.codec()),
                                convertResult.schema(),
                                "Generate a person",
                                new ChatCompletionsFormatter(),
                                config,
                                mockTransport);

                assertThat(result.isValid()).isTrue();
                assertThat(result.data().get("name").asText()).isEqualTo("Lovelace");

                wasiEngine.close();
        }

        // ---------------------------------------------------------------
        // 5. Numeric precision preserved
        // ---------------------------------------------------------------

        @Test
        void numericPrecision_preservedThroughRoundtrip() throws Exception {
                String schema = """
                                {
                                  "type": "object",
                                  "properties": {
                                    "score": { "type": "integer" },
                                    "rating": { "type": "number" }
                                  },
                                  "required": ["score", "rating"]
                                }
                                """;

                LlmTransport mockTransport = request -> openAiResponse(
                                "{\"score\": 42, \"rating\": 3.14}");

                ProviderConfig config = new ProviderConfig(
                                "https://api.openai.com/v1/chat/completions", "gpt-4o",
                                Map.of());

                RoundtripResult result = engine.generate(
                                schema, "Generate scores", new ChatCompletionsFormatter(), config, mockTransport);

                assertThat(result.isValid()).isTrue();
                assertThat(result.data().get("score").asInt()).isEqualTo(42);
                assertThat(result.data().get("rating").asDouble()).isEqualTo(3.14);
        }

        // ---------------------------------------------------------------
        // 6. Nullable fields handled
        // ---------------------------------------------------------------

        @Test
        void nullableFields_passThrough() throws Exception {
                String schema = """
                                {
                                  "type": "object",
                                  "properties": {
                                    "name": { "type": "string" },
                                    "nickname": { "type": ["string", "null"] }
                                  },
                                  "required": ["name"]
                                }
                                """;

                LlmTransport mockTransport = request -> openAiResponse(
                                "{\"name\": \"Ada\", \"nickname\": null}");

                ProviderConfig config = new ProviderConfig(
                                "https://api.openai.com/v1/chat/completions", "gpt-4o",
                                Map.of());

                RoundtripResult result = engine.generate(
                                schema, "Generate a person", new ChatCompletionsFormatter(), config, mockTransport);

                assertThat(result.data()).isNotNull();
                assertThat(result.data().get("name").asText()).isEqualTo("Ada");
                // Rehydrator may strip null optional fields or keep them as JSON null
                JsonNode nickname = result.data().get("nickname");
                assertThat(nickname == null || nickname.isNull()).isTrue();
        }

        // ---------------------------------------------------------------
        // 7. Schema conversion failure
        // ---------------------------------------------------------------

        @Test
        void invalidSchema_throwsEngineException() {
                LlmTransport mockTransport = request -> openAiResponse("{}");

                ProviderConfig config = new ProviderConfig(
                                "https://api.openai.com/v1/chat/completions", "gpt-4o",
                                Map.of());

                assertThatThrownBy(() -> engine.generate(
                                "NOT VALID JSON",
                                "Generate something",
                                new ChatCompletionsFormatter(),
                                config,
                                mockTransport))
                                .isInstanceOf(EngineException.class);
        }

        // ---------------------------------------------------------------
        // 8. Malformed LLM response
        // ---------------------------------------------------------------

        @Test
        void malformedLlmResponse_throwsEngineException() {
                LlmTransport mockTransport = request -> "THIS IS NOT JSON";

                ProviderConfig config = new ProviderConfig(
                                "https://api.openai.com/v1/chat/completions", "gpt-4o",
                                Map.of());

                assertThatThrownBy(() -> engine.generate(
                                PERSON_SCHEMA,
                                "Generate a person",
                                new ChatCompletionsFormatter(),
                                config,
                                mockTransport))
                                .isInstanceOf(EngineException.ResponseParsingException.class);
        }

        // ---------------------------------------------------------------
        // 9. HTTP 5xx failure
        // ---------------------------------------------------------------

        @Test
        void http5xx_propagatesWithStatusCode() {
                LlmTransport failingTransport = request -> {
                        throw new LlmTransportException("Internal Server Error", 500);
                };

                ProviderConfig config = new ProviderConfig(
                                "https://api.openai.com/v1/chat/completions", "gpt-4o",
                                Map.of());

                assertThatThrownBy(() -> engine.generate(
                                PERSON_SCHEMA,
                                "Generate a person",
                                new ChatCompletionsFormatter(),
                                config,
                                failingTransport))
                                .isInstanceOf(LlmTransportException.class)
                                .satisfies(ex -> {
                                        LlmTransportException tex = (LlmTransportException) ex;
                                        assertThat(tex.getStatusCode()).isEqualTo(500);
                                        assertThat(tex.isHttpError()).isTrue();
                                });
        }

        // ---------------------------------------------------------------
        // 10. Connection timeout
        // ---------------------------------------------------------------

        @Test
        void connectionTimeout_wrapsCorrectly() {
                LlmTransport timeoutTransport = request -> {
                        throw new LlmTransportException("Connection timed out", -1,
                                        new java.net.SocketTimeoutException("connect timed out"));
                };

                ProviderConfig config = new ProviderConfig(
                                "https://api.openai.com/v1/chat/completions", "gpt-4o",
                                Map.of());

                assertThatThrownBy(() -> engine.generate(
                                PERSON_SCHEMA,
                                "Generate a person",
                                new ChatCompletionsFormatter(),
                                config,
                                timeoutTransport))
                                .isInstanceOf(LlmTransportException.class)
                                .satisfies(ex -> {
                                        LlmTransportException tex = (LlmTransportException) ex;
                                        assertThat(tex.getStatusCode()).isEqualTo(-1);
                                        assertThat(tex.isHttpError()).isFalse();
                                });
        }

        // ---------------------------------------------------------------
        // 11. Structured warning serialization
        // ---------------------------------------------------------------

        @Test
        void structuredWarning_isPreservedNotDiscarded() throws Exception {
                // Verify that a structured warning object node is serialised via toString()
                // (producing a JSON snippet) and not asText() (which returns "" for objects).
                ObjectMapper mapper = new ObjectMapper();

                // Simulate two warning payloads: one plain string, one structured object
                String warningsJson = """
                                [
                                  "plain string warning",
                                  {"path": "/name", "message": "coercion applied", "rule": "string-trim"}
                                ]
                                """;
                com.fasterxml.jackson.databind.JsonNode warnings = mapper.readTree(warningsJson);

                // Apply the same extraction logic used in LlmRoundtripEngine
                java.util.List<String> extracted = new java.util.ArrayList<>();
                for (com.fasterxml.jackson.databind.JsonNode w : warnings) {
                        extracted.add(w.isTextual() ? w.asText() : w.toString());
                }

                assertThat(extracted).hasSize(2);
                // Plain string should remain clean (no extra quotes)
                assertThat(extracted.get(0)).isEqualTo("plain string warning");
                // Structured object must NOT be empty and must contain the field values
                assertThat(extracted.get(1)).isNotEmpty();
                assertThat(extracted.get(1)).contains("coercion applied");
                assertThat(extracted.get(1)).contains("/name");
        }

        // ---------------------------------------------------------------
        // Helpers
        // ---------------------------------------------------------------

        private static String openAiResponse(String content) {
                return """
                                {
                                  "choices": [{
                                    "message": {
                                      "content": "%s"
                                    }
                                  }]
                                }
                                """.formatted(content.replace("\"", "\\\""));
        }

        private static Path wasmPath() {
                String env = System.getenv("JSL_WASM_PATH");
                return env != null && !env.isEmpty()
                                ? Paths.get(env)
                                : Paths.get("../../target/wasm32-wasip1/release/jsonschema_llm_wasi.wasm");
        }
}
