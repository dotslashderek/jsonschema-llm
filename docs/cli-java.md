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

### Standalone (schema + codec access)

```java
import com.example.petstore.Pet;
import com.fasterxml.jackson.databind.JsonNode;

// Pre-loaded LLM-compatible schema
JsonNode llmSchema = Pet.schemaNode();

// Schema as string (for HTTP requests)
String schemaJson = Pet.SCHEMA_JSON;

// Rehydration codec
String codecJson = Pet.CODEC_JSON;
```

### With json-schema-llm-engine (full roundtrip)

Add the engine dependency to your `pom.xml`:

```xml
<dependency>
    <groupId>dev.jsonschema</groupId>
    <artifactId>json-schema-llm-engine</artifactId>
    <version>0.1.0</version>
</dependency>
```

```java
import com.example.petstore.Pet;
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

var result = Pet.generate("Generate a pet named Max", engine);

System.out.println(result.getData());     // Rehydrated data
System.out.println(result.isValid());     // JSON Schema validation result
System.out.println(result.getWarnings()); // Rehydration warnings
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
