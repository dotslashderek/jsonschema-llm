#!/usr/bin/env python3
"""Quick experiment: Do OpenAI description hints help with constraint adherence?"""

import json
import os
import sys
from openai import OpenAI

client = OpenAI(api_key=os.environ["OPENAI_API_KEY"])


def call_oai(
    schema: dict, label: str, prompt: str = "Generate realistic sample data"
) -> dict:
    """Call OpenAI with a schema and return parsed JSON."""
    print(f"\n{'=' * 60}")
    print(f"Experiment: {label}")
    print(f"Schema: {json.dumps(schema, indent=2)[:500]}")

    resp = client.chat.completions.create(
        model="gpt-4o-mini",
        response_format={
            "type": "json_schema",
            "json_schema": {"name": label, "schema": schema, "strict": True},
        },
        messages=[{"role": "user", "content": prompt}],
        temperature=0.8,
    )
    content = json.loads(resp.choices[0].message.content)
    print(f"Response: {json.dumps(content, indent=2)[:500]}")
    return content


# ── Experiment A: No hints (current behavior after p7 prunes constraints) ──
schema_no_hints = {
    "type": "object",
    "properties": {
        "age": {"type": "integer"},  # no min/max hints
        "score": {"type": "number"},  # no bounds
        "tags": {"type": "array", "items": {"type": "string"}},  # no maxItems
        "name": {"type": "string"},  # no maxLength
    },
    "required": ["age", "score", "tags", "name"],
    "additionalProperties": False,
}

result_a = call_oai(
    schema_no_hints,
    "A_no_hints",
    "Generate a person profile. Make age realistic, score from 0-100, up to 3 tags, name under 50 chars.",
)

# ── Experiment B: With description hints (proposed p7 enhancement) ──
schema_with_hints = {
    "type": "object",
    "properties": {
        "age": {
            "type": "integer",
            "description": "Must be between 0 and 150 (inclusive)",
        },
        "score": {
            "type": "number",
            "description": "Must be between 0.0 and 100.0 (inclusive)",
        },
        "tags": {
            "type": "array",
            "items": {"type": "string"},
            "description": "Maximum 3 items",
        },
        "name": {"type": "string", "description": "Maximum 50 characters"},
    },
    "required": ["age", "score", "tags", "name"],
    "additionalProperties": False,
}

result_b = call_oai(schema_with_hints, "B_with_hints", "Generate a person profile")

# Verify B respects constraints
checks_b = {
    "age_in_range": 0 <= result_b["age"] <= 150,
    "score_in_range": 0.0 <= result_b["score"] <= 100.0,
    "tags_max_3": len(result_b["tags"]) <= 3,
    "name_max_50": len(result_b["name"]) <= 50,
}
print(f"\nExperiment B constraint checks: {checks_b}")

# ── Experiment C: Opaque string with structural description (depth truncation) ──
schema_opaque = {
    "type": "object",
    "properties": {
        "config": {
            "type": "string",
            "description": "A JSON-encoded string. Structure: {host: string, port: integer, settings: {timeout: integer, retries: integer, ssl: boolean}}. Produce valid, parseable JSON.",
        },
    },
    "required": ["config"],
    "additionalProperties": False,
}

result_c = call_oai(schema_opaque, "C_opaque_string", "Generate a server configuration")

# Verify C produces valid parseable JSON in the string field
try:
    parsed_config = json.loads(result_c["config"])
    print(f"\nExperiment C: Successfully parsed opaque string!")
    print(f"Parsed config: {json.dumps(parsed_config, indent=2)}")

    # Check structure
    has_host = "host" in parsed_config
    has_port = "port" in parsed_config
    has_settings = "settings" in parsed_config
    print(f"Structure check: host={has_host}, port={has_port}, settings={has_settings}")
except json.JSONDecodeError as e:
    print(f"\nExperiment C: FAILED to parse opaque string: {e}")
    print(f"Raw value: {result_c['config'][:200]}")

# ── Experiment D: Opaque string WITH constraint hints in description ──
schema_opaque_constrained = {
    "type": "object",
    "properties": {
        "config": {
            "type": "string",
            "description": "A JSON-encoded string. Structure: {host: string, port: integer (1-65535), retries: integer (0-10), connections: [{id: string, timeout: integer (100-30000)}] (max 5 items)}. Produce valid, parseable JSON.",
        },
    },
    "required": ["config"],
    "additionalProperties": False,
}

result_d = call_oai(
    schema_opaque_constrained,
    "D_opaque_constrained",
    "Generate a realistic server configuration with multiple connections",
)

try:
    parsed_d = json.loads(result_d["config"])
    print(f"\nExperiment D: Parsed opaque string with constraints!")
    print(f"Parsed config: {json.dumps(parsed_d, indent=2)}")

    checks_d = {}
    if "port" in parsed_d:
        checks_d["port_in_range"] = 1 <= parsed_d["port"] <= 65535
    if "retries" in parsed_d:
        checks_d["retries_in_range"] = 0 <= parsed_d["retries"] <= 10
    if "connections" in parsed_d:
        checks_d["connections_max_5"] = len(parsed_d["connections"]) <= 5
        for i, conn in enumerate(parsed_d["connections"]):
            if "timeout" in conn:
                checks_d[f"conn_{i}_timeout"] = 100 <= conn["timeout"] <= 30000
    print(f"Constraint checks: {checks_d}")
except json.JSONDecodeError as e:
    print(f"\nExperiment D: FAILED to parse: {e}")

print("\n" + "=" * 60)
print("SUMMARY")
print("=" * 60)
print(f"A (no hints, prompt-guided): Generated, no constraint enforcement available")
print(f"B (description hints): {checks_b}")
print(f"C (opaque string): {'PASS - parsed' if 'parsed_config' in dir() else 'FAIL'}")
print(f"D (opaque + constraints): {checks_d if 'checks_d' in dir() else 'FAIL'}")
