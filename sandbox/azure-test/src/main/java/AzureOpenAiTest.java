import com.jsonschema.llm.engine.LlmRoundtripEngine;
import com.jsonschema.llm.engine.ProviderConfig;
import com.jsonschema.llm.engine.LlmTransport;
import com.jsonschema.llm.engine.LlmRequest;
import com.jsonschema.llm.engine.RoundtripResult;
import com.jsonschema.llm.engine.ChatCompletionsFormatter;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.net.URI;
import java.util.Map;

public class AzureOpenAiTest {
    public static void main(String[] args) throws Exception {
        String apiUrl = System.getenv("OPENAI_API_URL");
        String apiKey = System.getenv("OPENAI_API_KEY");

        if (apiUrl == null || apiKey == null) {
            System.err.println("Skipping test: OPENAI_API_URL or OPENAI_API_KEY environment variable is missing.");
            System.exit(0);
        }

        System.out.println("Testing Azure OpenAI at: " + apiUrl);

        ProviderConfig config = new ProviderConfig(apiUrl, "gpt-4o", Map.of(
                "api-key", apiKey,
                "Content-Type", "application/json"));

        HttpClient client = HttpClient.newHttpClient();
        LlmTransport transport = request -> {
            try {
                HttpRequest.Builder builder = HttpRequest.newBuilder()
                        .uri(URI.create(request.url()))
                        .POST(HttpRequest.BodyPublishers.ofString(request.body()));

                request.headers().forEach(builder::header);

                HttpResponse<String> response = client.send(builder.build(), HttpResponse.BodyHandlers.ofString());
                if (response.statusCode() >= 300) {
                    throw new RuntimeException("HTTP " + response.statusCode() + ": " + response.body());
                }
                return response.body();
            } catch (Exception e) {
                throw new RuntimeException("Transport failed", e);
            }
        };

        try (LlmRoundtripEngine engine = LlmRoundtripEngine.create(new ChatCompletionsFormatter(), config, transport)) {
            System.out.println("--------------------------------------------------");
            testArazzo(engine);
            System.out.println("--------------------------------------------------");
            testOas31(engine);
            System.out.println("--------------------------------------------------");
            testAsyncApi(engine);
            System.out.println("--------------------------------------------------");
            System.out.println("✅ All tests passed!");
        }
    }

    private static void testArazzo(LlmRoundtripEngine engine) throws Exception {
        System.out.println("Generating Arazzo Information snippet...");
        String prompt = "Generate an arazzo Information snippet about an API description document authored by Alice.";
        RoundtripResult result = com.example.arazzo.SchemaGenerator.generate(
                com.example.arazzo.SchemaGenerator.Component.from("infO"), prompt, engine);
        System.out.println("Arazzo generation successful!");
        System.out.println("Valid: " + result.isValid());
        System.out.println(result.data().toPrettyString());
    }

    private static void testOas31(LlmRoundtripEngine engine) throws Exception {
        System.out.println("Generating OAS 3.1 Contact snippet...");
        String prompt = "Generate a contact object for Jane Doe jane.doe@example.com at url https://example.com.";
        RoundtripResult result = com.example.oas31.SchemaGenerator.generate(
                com.example.oas31.SchemaGenerator.Component.from("CONTACT"), prompt, engine);
        System.out.println("OAS 3.1 generation successful!");
        System.out.println("Valid: " + result.isValid());
        System.out.println(result.data().toPrettyString());
    }

    private static void testAsyncApi(LlmRoundtripEngine engine) throws Exception {
        System.out.println("Generating AsyncAPI Contact snippet...");
        String prompt = "Generate an AsyncAPI contact object for Jane Doe jane@example.com.";
        RoundtripResult result = com.example.asyncapi.SchemaGenerator.generate(
                com.example.asyncapi.SchemaGenerator.Component.from("cOnTacT"), prompt, engine);
        System.out.println("AsyncAPI generation successful!");
        System.out.println("Valid: " + result.isValid());
        System.out.println(result.data().toPrettyString());
    }
}
