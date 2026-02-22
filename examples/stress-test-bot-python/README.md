# Stress Test Bot — Python

Stress test client for json-schema-llm Python (PyO3) bindings. Mirrors the TS reference client (`examples/stress-test-bot/src/index.ts`).

## Pipeline

`convert` → OpenAI structured output → `rehydrate` → validate against original schema

## Prerequisites

1. **Build the Python binding** (from repo root):

   ```bash
   cd crates/json-schema-llm-python && maturin develop && cd ../..
   ```

2. **Install dependencies**:

   ```bash
   pip install -r examples/stress-test-bot-python/requirements.txt
   ```

3. **Set your OpenAI API key**:

   ```bash
   export OPENAI_API_KEY="sk-..."
   ```

## Usage

```bash
python examples/stress-test-bot-python/main.py [OPTIONS]
```

### Options

| Flag            | Default              | Description                           |
| --------------- | -------------------- | ------------------------------------- |
| `--count N`     | 5                    | Number of schemas to test             |
| `--seed N`      | random               | Random seed for reproducible ordering |
| `--model NAME`  | gpt-4o-mini          | OpenAI model name                     |
| `--schemas-dir` | tests/schemas/stress | Directory containing JSON schemas     |

### Examples

```bash
# Test 3 schemas with a fixed seed
python examples/stress-test-bot-python/main.py --count 3 --seed 42

# Test against real-world schemas
python examples/stress-test-bot-python/main.py --schemas-dir tests/schemas/real-world --count 5

# Use a different model
python examples/stress-test-bot-python/main.py --model gpt-4o --count 2
```

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

## Tests

```bash
# Build binding first, then run tests
cd crates/json-schema-llm-python && maturin develop && cd ../..
cd examples/stress-test-bot-python && python -m pytest test_client.py -v
```
