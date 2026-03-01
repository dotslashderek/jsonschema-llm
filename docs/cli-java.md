# Java SDK — CLI Walkthrough

End-to-end guide: convert a JSON Schema, generate a typed Java SDK, and use it in your application.

## Prerequisites

- `json-schema-llm` CLI ([install](../README.md#quick-start))
- Java 17+
- Maven 3.8+

---

## Step 1: Convert your schema

Convert a JSON Schema to an LLM-compatible form with per-component schemas and a manifest:

```bash
json-schema-llm convert my-schema.json \
  --output-dir ./converted \
  --target openai-strict
```

This produces:

- `manifest.json` — component listing and metadata
- Per-component `schema.json` / `codec.json` pairs

> **Tip**: Use `--target gemini` or `--target claude` for other providers. Use `--mode permissive` to skip strict enforcement.

---

## Step 2: Generate the SDK

```bash
json-schema-llm gen-sdk \
  --language java \
  --schema ./converted \
  --package com.example.petstore \
  --output ./my-sdk
```

This generates a Maven project:

```
my-sdk/
├── pom.xml
└── src/main/
    ├── java/com/example/petstore/
    │   ├── Generator.java       # Facade with all components
    │   ├── Pet.java             # Per-component class
    │   └── Owner.java
    └── resources/schemas/
        ├── $defs/Pet/schema.json
        ├── $defs/Pet/codec.json
        └── ...
```

---

## Step 3: Build the SDK

```bash
cd my-sdk
mvn compile
```

To install to your local Maven repository:

```bash
mvn install
```

---

## Step 4: Use the SDK

### Unified generator (recommended for generic operations)

Use the `Component` enum and `SchemaGenerator` for dynamic dispatch — ideal when the component is determined at runtime (e.g. from user input, HTTP request body, or config).

```java
import com.example.petstore.SchemaGenerator;
import com.example.petstore.SchemaGenerator.Component;
import dev.jsonschema.llm.engine.*;

var engine = LlmRoundtripEngine.create(
    new OpenAIFormatter(),
    new ProviderConfig(
        "https://api.openai.com/v1/chat/completions",
        "gpt-4o",
        Map.of("Authorization", "Bearer YOUR_API_KEY")
    ),
    new HttpClientTransport()
);

// Generate any component by enum
var result = SchemaGenerator.generate(Component.PET, "Generate a pet named Max", engine);

// Dynamic lookup from a string
Component component = Component.from("Pet");
var result2 = SchemaGenerator.generate(component, "Generate a Pet", engine);

// With JSON Patch for schema customization
List<JsonPatchOp> patches = List.of(
    new JsonPatchOp.Add("/properties/color", Map.of("type", "string"))
);
var result3 = SchemaGenerator.generate(Component.PET, "Generate a pet", engine, patches);

System.out.println(result.getData());     // Rehydrated data
System.out.println(result.isValid());     // JSON Schema validation result
System.out.println(result.getWarnings()); // Rehydration warnings
```

### Focused component access

Import a specific component class directly when you know exactly which component you need at compile time.

```java
import com.example.petstore.Pet;
import dev.jsonschema.llm.engine.*;

var engine = LlmRoundtripEngine.create(/* ... */);

// Direct generation
var result = Pet.generate("Generate a pet named Max", engine);

// Access individual artifacts
var schema   = Pet.schema();     // LLM-compatible schema (JsonNode)
var codec    = Pet.codec();      // Rehydration codec
var original = Pet.original();   // Original JSON Schema
```

### Low-level access (schema + codec only)

```java
import com.example.petstore.Pet;
import com.fasterxml.jackson.databind.JsonNode;

JsonNode llmSchema = Pet.schema();
JsonNode codec     = Pet.codec();
// Use with your own LLM integration
```

---

## Reference

| CLI Flag             | Description                                                       |
| -------------------- | ----------------------------------------------------------------- |
| `--language java`    | Target language                                                   |
| `--schema <dir>`     | Directory with `manifest.json` (output of `convert --output-dir`) |
| `--package <name>`   | Java package name (e.g. `com.example.petstore`)                   |
| `--output <dir>`     | Output directory for generated project                            |
| `--build-tool maven` | Build tool (default for Java)                                     |
| `--git-init`         | Initialize a git repository in output dir                         |
