package com.jsonschema.llm.wasi;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.MethodSource;

import java.io.File;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.*;

class ConformanceTest {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private static final String FIXTURES_PATH = "../../tests/conformance/fixtures.json";
    private JsonSchemaLlmWasi engine;

    @BeforeEach
    void setUp() {
        engine = new JsonSchemaLlmWasi();
    }

    @AfterEach
    void tearDown() {
        engine.close();
    }

    // -----------------------------------------------------------------------
    // Fixture loading
    // -----------------------------------------------------------------------

    private static JsonNode loadFixtures() throws IOException {
        return MAPPER.readTree(new File(FIXTURES_PATH));
    }

    static Stream<String> convertFixtureIds() throws IOException {
        JsonNode fixtures = loadFixtures();
        List<String> ids = new ArrayList<>();
        for (JsonNode fx : fixtures.get("suites").get("convert").get("fixtures")) {
            ids.add(fx.get("id").asText());
        }
        return ids.stream();
    }

    static Stream<String> roundtripFixtureIds() throws IOException {
        JsonNode fixtures = loadFixtures();
        List<String> ids = new ArrayList<>();
        for (JsonNode fx : fixtures.get("suites").get("roundtrip").get("fixtures")) {
            ids.add(fx.get("id").asText());
        }
        return ids.stream();
    }

    static Stream<String> rehydrateErrorFixtureIds() throws IOException {
        JsonNode fixtures = loadFixtures();
        List<String> ids = new ArrayList<>();
        for (JsonNode fx : fixtures.get("suites").get("rehydrate_error").get("fixtures")) {
            ids.add(fx.get("id").asText());
        }
        return ids.stream();
    }

    private static JsonNode getFixture(String suite, String fixtureId) throws IOException {
        JsonNode fixtures = loadFixtures();
        for (JsonNode fx : fixtures.get("suites").get(suite).get("fixtures")) {
            if (fx.get("id").asText().equals(fixtureId)) {
                return fx;
            }
        }
        throw new IllegalArgumentException("Fixture not found: " + fixtureId);
    }

    // -----------------------------------------------------------------------
    // Convert suite
    // -----------------------------------------------------------------------

    @ParameterizedTest(name = "{0}")
    @MethodSource("convertFixtureIds")
    void conformanceConvert(String fixtureId) throws Exception {
        JsonNode fx = getFixture("convert", fixtureId);
        JsonNode input = fx.get("input");
        JsonNode expected = fx.get("expected");

        // Error case: schema_raw → raw FFI
        if (input.has("schema_raw")) {
            assertTrue(expected.path("is_error").asBoolean());

            String optsJson = input.has("options") ? MAPPER.writeValueAsString(input.get("options")) : "{}";

            JsonSchemaLlmWasi.JslException thrown = assertThrows(
                    JsonSchemaLlmWasi.JslException.class,
                    () -> engine.callJsl("jsl_convert", input.get("schema_raw").asText(), optsJson));

            if (expected.has("error_has_keys")) {
                for (JsonNode key : expected.get("error_has_keys")) {
                    String k = key.asText();
                    if ("code".equals(k))
                        assertNotNull(thrown.getCode());
                    if ("message".equals(k))
                        assertNotNull(thrown.getMessage());
                }
            }

            if (expected.has("error_code")) {
                assertEquals(expected.get("error_code").asText(), thrown.getCode());
            }
            return;
        }

        // Normal convert
        ObjectNode optionsNode = input.has("options") && input.get("options").size() > 0
                ? (ObjectNode) input.get("options")
                : null;

        JsonNode result = engine.convert(input.get("schema"), optionsNode);
        assertConvertExpected(result, expected);
    }

    private void assertConvertExpected(JsonNode result, JsonNode expected) {
        if (expected.has("has_keys")) {
            for (JsonNode key : expected.get("has_keys")) {
                assertTrue(result.has(key.asText()), "result missing key: " + key.asText());
            }
        }

        if (expected.has("apiVersion")) {
            assertEquals(expected.get("apiVersion").asText(), result.get("apiVersion").asText());
        }

        if (expected.has("schema_has_properties")) {
            assertTrue(result.get("schema").has("properties"));
        }

        if (expected.has("codec_has_schema_uri")) {
            assertNotNull(result.get("codec"));
        }
    }

    // -----------------------------------------------------------------------
    // Roundtrip suite
    // -----------------------------------------------------------------------

    @ParameterizedTest(name = "{0}")
    @MethodSource("roundtripFixtureIds")
    void conformanceRoundtrip(String fixtureId) throws Exception {
        JsonNode fx = getFixture("roundtrip", fixtureId);
        JsonNode input = fx.get("input");
        JsonNode expected = fx.get("expected");

        ObjectNode optionsNode = input.has("options") && input.get("options").size() > 0
                ? (ObjectNode) input.get("options")
                : null;

        JsonNode convertResult = engine.convert(input.get("schema"), optionsNode);
        JsonNode codec = convertResult.get("codec");

        JsonNode rehydrateResult = engine.rehydrate(input.get("data"), codec, input.get("schema"));

        if (expected.has("has_keys")) {
            for (JsonNode key : expected.get("has_keys")) {
                assertTrue(rehydrateResult.has(key.asText()),
                        "result missing key: " + key.asText());
            }
        }

        if (expected.has("apiVersion")) {
            assertEquals(expected.get("apiVersion").asText(),
                    rehydrateResult.get("apiVersion").asText());
        }

        if (expected.has("data")) {
            assertEquals(expected.get("data"), rehydrateResult.get("data"));
        }

        if (expected.has("data_user_name")) {
            assertEquals(expected.get("data_user_name").asText(),
                    rehydrateResult.get("data").get("user").get("name").asText());
        }

        if (expected.has("data_value")) {
            assertEquals(expected.get("data_value").doubleValue(),
                    rehydrateResult.get("data").get("value").doubleValue(), 0.001);
        }

        if (expected.has("warnings_is_array")) {
            assertTrue(rehydrateResult.has("warnings"));
            assertTrue(rehydrateResult.get("warnings").isArray());
        }
    }

    // -----------------------------------------------------------------------
    // Rehydrate error suite
    // -----------------------------------------------------------------------

    @ParameterizedTest(name = "{0}")
    @MethodSource("rehydrateErrorFixtureIds")
    void conformanceRehydrateError(String fixtureId) throws Exception {
        JsonNode fx = getFixture("rehydrate_error", fixtureId);
        JsonNode input = fx.get("input");
        JsonNode expected = fx.get("expected");

        assertTrue(expected.path("is_error").asBoolean());

        String dataJson = MAPPER.writeValueAsString(input.get("data"));
        String schemaJson = MAPPER.writeValueAsString(input.get("schema"));
        String codecArg = input.has("codec_raw") ? input.get("codec_raw").asText() : "{}";

        JsonSchemaLlmWasi.JslException thrown = assertThrows(
                JsonSchemaLlmWasi.JslException.class,
                () -> engine.callJsl("jsl_rehydrate", dataJson, codecArg, schemaJson));

        if (expected.has("error_has_keys")) {
            for (JsonNode key : expected.get("error_has_keys")) {
                String k = key.asText();
                if ("code".equals(k))
                    assertNotNull(thrown.getCode());
                if ("message".equals(k))
                    assertNotNull(thrown.getMessage());
            }
        }
    }
}
