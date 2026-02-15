# Stress Test Bot — Java

Stress test client for jsonschema-llm Java (Panama FFM / JNI) bindings. Mirrors the TS reference client (`examples/stress-test-bot/src/index.ts`).

## Pipeline

`convert` → OpenAI structured output → `rehydrate` → validate against original schema

## Prerequisites

1. **JDK 17+** (JDK 22+ recommended for Panama FFM, JNI fallback on JDK 17–21)
2. **Rust toolchain** (for building native library)
3. **Set your OpenAI API key**:

   ```bash
   export OPENAI_API_KEY="sk-..."
   ```

## Usage

From the repository root:

```bash
cd examples/stress-test-bot-java
./gradlew run --args="[OPTIONS]"
```

### Options

| Flag            | Default              | Description                           |
| --------------- | -------------------- | ------------------------------------- |
| `--count N`     | 5                    | Number of schemas to test             |
| `--seed N`      | random               | Random seed for reproducible ordering |
| `--model NAME`  | gpt-4o-mini          | OpenAI model name                     |
| `--schemas-dir` | tests/schemas/stress | Directory containing JSON schemas     |
| `--help`        |                      | Show help message                     |

### Examples

```bash
# Test 3 schemas with a fixed seed
./gradlew run --args="--count 3 --seed 42"

# Test against real-world schemas
./gradlew run --args="--schemas-dir ../../tests/schemas/real-world --count 5"

# Use a different model
./gradlew run --args="--model gpt-4o --count 2"
```

## Build Details

The Gradle project:

- Uses `includeBuild("../../bindings/java")` to resolve the binding JAR locally
- Automatically triggers `cargo build --release -p jsonschema-llm-java` before compilation
- Enables `--enable-native-access=ALL-UNNAMED` for Panama FFM support

## Output

Each schema shows `✅` or `❌` with timing:

```
Testing 3/53 schemas (model=gpt-4o-mini, seed=42)

=== Testing combo_poly_mix_3.json ===
  converting...
  calling gpt-4o-mini...
  rehydrating...
  ✅ Validated against original schema
  ✅ Success! Rehydrated data: object(5 keys)
  ⏱  1.23s

Summary: 3/3 passed (4.56s total).
```

Exit code is `0` if all schemas pass, `1` if any fail.
