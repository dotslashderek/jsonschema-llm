# Python SDK — CLI Walkthrough

End-to-end guide: convert a JSON Schema, generate a typed Python SDK, and use it in your application.

## Prerequisites

- `json-schema-llm` CLI ([install](../README.md#quick-start))
- Python 3.10+
- `pip` or `uv`

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
  --language python \
  --schema ./converted \
  --package my-petstore-sdk \
  --output ./my-sdk
```

This generates a pip-installable Python package:

```
my-sdk/
├── pyproject.toml
├── my_petstore_sdk/
│   ├── __init__.py
│   ├── generator.py        # Barrel module with all components
│   ├── pet.py              # Per-component module
│   ├── owner.py
│   └── schemas/            # Embedded schema + codec JSON files
│       ├── $defs/Pet/schema.json
│       ├── $defs/Pet/codec.json
│       └── ...
```

---

## Step 3: Install the SDK

```bash
cd my-sdk
pip install -e .
```

---

## Step 4: Use the SDK

### Standalone (schema + codec access)

```python
from my_petstore_sdk import pet

# Pre-loaded LLM-compatible schema
llm_schema = pet.SCHEMA

# Rehydration codec
codec = pet.CODEC
```

### With json-schema-llm-engine (full roundtrip)

```bash
pip install json-schema-llm-engine
```

```python
import json
from my_petstore_sdk import pet
from json_schema_llm_engine import LlmRoundtripEngine

engine = LlmRoundtripEngine(
    formatter=OpenAIFormatter(),  # e.g. from json_schema_llm_engine.formatters
    config={
        "url": "https://api.openai.com/v1/chat/completions",
        "model": "gpt-4o",
        "headers": {"Authorization": "Bearer YOUR_API_KEY"},
    },
    transport=RequestsTransport(),  # e.g. from json_schema_llm_engine.transports
)

result = pet.generate("Generate a pet named Max", engine)

print(result.data)           # Rehydrated data in original schema shape
print(result.is_valid)       # True if passes JSON Schema validation
print(result.warnings)       # Rehydration warnings (e.g. dropped nulls)
```

---

## Reference

| CLI Flag                  | Description                                                       |
| ------------------------- | ----------------------------------------------------------------- |
| `--language python`       | Target language                                                   |
| `--schema <dir>`          | Directory with `manifest.json` (output of `convert --output-dir`) |
| `--package <name>`        | Python package name (PEP 508 format)                              |
| `--output <dir>`          | Output directory for generated project                            |
| `--build-tool setuptools` | Build tool (default for Python)                                   |
| `--git-init`              | Initialize a git repository in output dir                         |
