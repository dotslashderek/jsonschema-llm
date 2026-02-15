package com.jsonschema.llm.stress;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.jsonschema.llm.ConvertOptions;
import com.jsonschema.llm.ConvertResult;
import com.jsonschema.llm.JsonSchemaLlm;
import com.jsonschema.llm.JsonSchemaLlmException;
import com.jsonschema.llm.RehydrateResult;
import com.networknt.schema.JsonSchema;
import com.networknt.schema.JsonSchemaFactory;
import com.networknt.schema.SpecVersion;
import com.networknt.schema.ValidationMessage;
import com.openai.client.OpenAIClient;
import com.openai.client.okhttp.OpenAIOkHttpClient;
import com.openai.core.JsonValue;
import com.openai.models.ResponseFormatJsonSchema;
import com.openai.models.chat.completions.ChatCompletion;
import com.openai.models.chat.completions.ChatCompletionCreateParams;
import com.openai.models.chat.completions.ChatCompletionSystemMessageParam;
import com.openai.models.chat.completions.ChatCompletionUserMessageParam;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;

/**
 * Stress test bot for jsonschema-llm Java bindings.
 *
 * <p>
 * Mirrors the TS reference client (examples/stress-test-bot/src/index.ts).
 * Pipeline: convert → OpenAI structured output → rehydrate → validate.
 */
public class StressTestBot {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private static final JsonSchemaFactory SCHEMA_FACTORY = JsonSchemaFactory
            .getInstance(SpecVersion.VersionFlag.V202012);

    public static void main(String[] args) throws Exception {
        int count = 5;
        Integer seed = null;
        String model = "gpt-4o-mini";
        String schemasDir = null;

        for (int i = 0; i < args.length; i++) {
            switch (args[i]) {
                case "--count" -> count = Integer.parseInt(args[++i]);
                case "--seed" -> seed = Integer.parseInt(args[++i]);
                case "--model" -> model = args[++i];
                case "--schemas-dir" -> schemasDir = args[++i];
                case "--help" -> {
                    printHelp();
                    return;
                }
                default -> {
                    System.err.println("Unknown argument: " + args[i]);
                    printHelp();
                    System.exit(1);
                }
            }
        }

        if (count < 1) {
            System.err.println("Error: --count must be a positive integer");
            System.exit(1);
        }

        Path schemaPath = resolveSchemaDir(schemasDir);
        List<String> allFiles = loadSchemaFiles(schemaPath);

        List<String> testFiles = fisherYatesShuffle(allFiles, seed)
                .subList(0, Math.min(count, allFiles.size()));
        System.out.printf(
                "Testing %d/%d schemas (model=%s, seed=%s)%n",
                testFiles.size(), allFiles.size(), model, seed != null ? seed : "random");

        OpenAIClient client = OpenAIOkHttpClient.fromEnv();

        int passed = 0;
        double totalTime = 0;
        for (String file : testFiles) {
            TestResult result = testSchema(file, schemaPath, client, model);
            totalTime += result.elapsedSeconds;
            if (result.passed) {
                passed++;
            }
        }

        System.out.printf(
                "%n%nSummary: %d/%d passed (%.2fs total).%n", passed, testFiles.size(), totalTime);
        if (passed < testFiles.size()) {
            System.exit(1);
        }
    }

    private static TestResult testSchema(
            String filename, Path schemasDir, OpenAIClient client, String model) {
        System.out.printf("%n=== Testing %s ===%n", filename);
        long startTime = System.nanoTime();

        try {
            Path schemaPath = schemasDir.resolve(filename);
            JsonNode originalSchema = MAPPER.readTree(Files.readString(schemaPath));

            System.out.println("  converting...");
            ConvertOptions options = ConvertOptions.builder()
                    .target(ConvertOptions.Target.OPENAI_STRICT)
                    .polymorphism(ConvertOptions.PolymorphismStrategy.ANY_OF)
                    .maxDepth(50)
                    .recursionLimit(3)
                    .build();

            ConvertResult convertResult = JsonSchemaLlm.convert(originalSchema, options);

            String schemaName = "stress_test";

            System.out.printf("  calling %s...%n", model);

            ChatCompletionCreateParams params = ChatCompletionCreateParams.builder()
                    .model(model)
                    .addMessage(ChatCompletionSystemMessageParam.builder()
                            .content("You are a helpful assistant. Generate valid JSON.")
                            .build())
                    .addMessage(ChatCompletionUserMessageParam.builder()
                            .content("Generate a complex example.")
                            .build())
                    .responseFormat(ResponseFormatJsonSchema.builder()
                            .jsonSchema(ResponseFormatJsonSchema.JsonSchema.builder()
                                    .name(schemaName)
                                    .schema(ResponseFormatJsonSchema.JsonSchema.Schema.builder()
                                            .putAllAdditionalProperties(
                                                    jsonNodeToMap(convertResult.schema()))
                                            .build())
                                    .strict(true)
                                    .build())
                            .build())
                    .build();

            ChatCompletion completion = client.chat().completions().create(params);
            String rawContent = completion.choices().get(0).message().content().orElse(null);

            if (rawContent == null || rawContent.isEmpty()) {
                throw new RuntimeException("No content from OpenAI");
            }

            JsonNode llmData = MAPPER.readTree(rawContent);

            System.out.println("  rehydrating...");
            RehydrateResult rehydrateResult = JsonSchemaLlm.rehydrate(llmData, convertResult.codec(), originalSchema);

            if (rehydrateResult.warnings() != null && !rehydrateResult.warnings().isEmpty()) {
                System.out.printf("  Warnings: %s%n", rehydrateResult.warnings());
            }

            JsonSchema validator = SCHEMA_FACTORY.getSchema(originalSchema);
            Set<ValidationMessage> errors = validator.validate(rehydrateResult.data());

            double elapsed = (System.nanoTime() - startTime) / 1_000_000_000.0;

            if (!errors.isEmpty()) {
                System.out.printf("  ❌ Validation failed: %s%n", errors);
                System.out.printf("  ⏱  %.2fs%n", elapsed);
                return new TestResult(false, elapsed);
            }

            System.out.println("  ✅ Validated against original schema");
            System.out.printf(
                    "  ✅ Success! Rehydrated data: %s%n", describeData(rehydrateResult.data()));
            System.out.printf("  ⏱  %.2fs%n", elapsed);
            return new TestResult(true, elapsed);

        } catch (JsonSchemaLlmException e) {
            double elapsed = (System.nanoTime() - startTime) / 1_000_000_000.0;
            System.out.printf("  ❌ FAIL: %s%n", e.getMessage());
            if (e.getCode() != null)
                System.out.printf("     Code: %s%n", e.getCode());
            if (e.getPath() != null)
                System.out.printf("     Path: %s%n", e.getPath());
            System.out.printf("  ⏱  %.2fs%n", elapsed);
            return new TestResult(false, elapsed);
        } catch (Exception e) {
            double elapsed = (System.nanoTime() - startTime) / 1_000_000_000.0;
            System.out.printf("  ❌ FAIL: %s%n", e.getMessage());
            System.out.printf("  ⏱  %.2fs%n", elapsed);
            return new TestResult(false, elapsed);
        }
    }

    private static Path resolveSchemaDir(String schemasDir) {
        if (schemasDir != null) {
            return Path.of(schemasDir).toAbsolutePath();
        }

        Path dir = Path.of("").toAbsolutePath();
        for (int i = 0; i < 10; i++) {
            if (Files.isRegularFile(dir.resolve("Cargo.toml"))) {
                return dir.resolve("tests/schemas/stress");
            }
            dir = dir.getParent();
            if (dir == null)
                break;
        }

        System.err.println("Error: could not locate repo root. Use --schemas-dir.");
        System.exit(1);
        return null;
    }

    private static List<String> loadSchemaFiles(Path schemasDir) throws IOException {
        if (!Files.isDirectory(schemasDir)) {
            System.err.printf("Error: schema directory not found: %s%n", schemasDir);
            System.exit(1);
        }

        List<String> files = new ArrayList<>();
        try (var stream = Files.list(schemasDir)) {
            stream.filter(p -> p.toString().endsWith(".json"))
                    .map(p -> p.getFileName().toString())
                    .sorted()
                    .forEach(files::add);
        }

        if (files.isEmpty()) {
            System.err.printf("Error: no .json files found in %s%n", schemasDir);
            System.exit(1);
        }

        return files;
    }

    /**
     * Fisher-Yates shuffle with mulberry32 PRNG.
     * Matches the TS reference exactly for cross-language seed reproducibility.
     */
    private static List<String> fisherYatesShuffle(List<String> arr, Integer seed) {
        List<String> copy = new ArrayList<>(arr);
        int[] state;
        if (seed != null) {
            state = new int[] { seed };
        } else {
            state = new int[] { (int) (Math.random() * Integer.MAX_VALUE) };
        }

        for (int i = copy.size() - 1; i > 0; i--) {
            int j = (int) Math.floor(mulberry32(state) * (i + 1));
            String tmp = copy.get(i);
            copy.set(i, copy.get(j));
            copy.set(j, tmp);
        }
        return copy;
    }

    /** Mulberry32 PRNG — faithful port of the TS reference implementation. */
    private static double mulberry32(int[] state) {
        state[0] = state[0] + 0x6D2B79F5;
        int s = state[0];
        // Java int arithmetic naturally truncates to 32 bits, matching JS Math.imul
        int t = (s ^ (s >>> 15)) * (1 | s);
        t = (t + ((t ^ (t >>> 7)) * (61 | t)) ^ t);
        return Integer.toUnsignedLong(t ^ (t >>> 14)) / 4294967296.0;
    }

    private static String describeData(JsonNode data) {
        if (data == null || data.isNull())
            return "null";
        if (data.isArray())
            return String.format("array(%d items)", data.size());
        if (data.isObject())
            return String.format("object(%d keys)", data.size());
        String text = data.asText();
        return String.format("%s: %s", data.getNodeType(),
                text.length() > 50 ? text.substring(0, 50) : text);
    }

    private static Map<String, JsonValue> jsonNodeToMap(JsonNode node) {
        Map<String, JsonValue> map = new LinkedHashMap<>();
        node.fields().forEachRemaining(entry -> map.put(entry.getKey(), jsonNodeToJsonValue(entry.getValue())));
        return map;
    }

    private static JsonValue jsonNodeToJsonValue(JsonNode node) {
        if (node.isNull())
            return JsonValue.from(null);
        if (node.isBoolean())
            return JsonValue.from(node.booleanValue());
        if (node.isInt() || node.isLong())
            return JsonValue.from(node.longValue());
        if (node.isDouble() || node.isFloat())
            return JsonValue.from(node.doubleValue());
        if (node.isTextual())
            return JsonValue.from(node.textValue());
        if (node.isArray()) {
            List<JsonValue> list = new ArrayList<>();
            node.forEach(elem -> list.add(jsonNodeToJsonValue(elem)));
            return JsonValue.from(list);
        }
        if (node.isObject()) {
            Map<String, JsonValue> map = new LinkedHashMap<>();
            node.fields().forEachRemaining(entry -> map.put(entry.getKey(), jsonNodeToJsonValue(entry.getValue())));
            return JsonValue.from(map);
        }
        return JsonValue.from(node.asText());
    }

    private static void printHelp() {
        System.out.println("Usage: stress-test-bot-java [OPTIONS]");
        System.out.println();
        System.out.println("Options:");
        System.out.println("  --count N        Number of schemas to test (default: 5)");
        System.out.println("  --seed N         Random seed for reproducible ordering");
        System.out.println("  --model NAME     OpenAI model name (default: gpt-4o-mini)");
        System.out.println("  --schemas-dir    Directory containing JSON schemas");
        System.out.println("  --help           Show this help message");
    }

    private record TestResult(boolean passed, double elapsedSeconds) {
    }
}
