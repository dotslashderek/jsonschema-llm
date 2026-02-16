#!/usr/bin/env python3
"""Stress test bot for jsonschema-llm WASI wrapper.

Mirrors the TS reference client (examples/stress-test-bot/src/index.ts).
Pipeline: convert → OpenAI structured output → rehydrate → validate.
"""

import argparse
import json
import os
import sys
import time
from typing import Optional

# Add WASI wrapper to path
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "bindings", "python-wasi"))
from jsonschema_llm_wasi import Engine, JslError  # noqa: E402
from openai import OpenAI
import jsonschema
from jsonschema import Draft202012Validator


def parse_args() -> argparse.Namespace:
    """Parse CLI arguments matching TS client interface."""
    parser = argparse.ArgumentParser(
        description="Stress test bot for jsonschema-llm Python bindings"
    )
    parser.add_argument(
        "--count",
        type=int,
        default=5,
        help="Number of schemas to test (default: 5)",
    )
    parser.add_argument(
        "--seed",
        type=int,
        default=None,
        help="Random seed for reproducible schema ordering",
    )
    parser.add_argument(
        "--model",
        type=str,
        default="gpt-4o-mini",
        help="OpenAI model name (default: gpt-4o-mini)",
    )
    parser.add_argument(
        "--schemas-dir",
        type=str,
        default=None,
        help="Directory containing JSON schemas (default: tests/schemas/stress)",
    )
    args = parser.parse_args()

    if args.count < 1:
        parser.error("--count must be a positive integer")

    return args


def _to_int32(x: int) -> int:
    """Truncate to signed 32-bit integer (JS `|0` operator)."""
    x = x & 0xFFFFFFFF
    return x - 0x100000000 if x >= 0x80000000 else x


def _unsigned_rshift(val: int, n: int) -> int:
    """JS `>>>` operator: unsigned right shift."""
    return (val & 0xFFFFFFFF) >> n


def _imul(a: int, b: int) -> int:
    """JS `Math.imul`: 32-bit truncating multiply."""
    return _to_int32((a & 0xFFFFFFFF) * (b & 0xFFFFFFFF))


def _mulberry32(seed: int) -> callable:
    """Mulberry32 PRNG — bit-exact port of the TS reference."""
    s = _to_int32(seed)

    def random() -> float:
        nonlocal s
        s = _to_int32(s + 0x6D2B79F5)
        t = _imul(s ^ _unsigned_rshift(s, 15), 1 | s)
        t = _to_int32(t + _imul(t ^ _unsigned_rshift(t, 7), 61 | t) ^ t)
        return _unsigned_rshift(t ^ _unsigned_rshift(t, 14), 0) / 4294967296

    return random


def fisher_yates_shuffle(arr: list[str], seed: Optional[int] = None) -> list[str]:
    """Fisher-Yates shuffle with mulberry32 PRNG. Matches TS reference."""
    import math
    import random as _random

    copy = list(arr)
    if seed is not None:
        rng = _mulberry32(seed)
    else:
        rng = _random.random

    for i in range(len(copy) - 1, 0, -1):
        j = math.floor(rng() * (i + 1))
        copy[i], copy[j] = copy[j], copy[i]
    return copy


def resolve_schemas_dir(schemas_dir: Optional[str]) -> str:
    """Resolve schema directory, defaulting to tests/schemas/stress from repo root."""
    if schemas_dir:
        return os.path.abspath(schemas_dir)

    # Walk up to find repo root (contains Cargo.toml)
    d = os.path.dirname(os.path.abspath(__file__))
    for _ in range(10):
        if os.path.isfile(os.path.join(d, "Cargo.toml")):
            return os.path.join(d, "tests", "schemas", "stress")
        d = os.path.dirname(d)

    print("Error: could not locate repo root. Use --schemas-dir.", file=sys.stderr)
    sys.exit(1)


def load_schema_files(schemas_dir: str) -> list[str]:
    """Load and return sorted list of .json files in the schemas directory."""
    if not os.path.isdir(schemas_dir):
        print(f"Error: schema directory not found: {schemas_dir}", file=sys.stderr)
        sys.exit(1)

    files = sorted(f for f in os.listdir(schemas_dir) if f.endswith(".json"))
    if not files:
        print(f"Error: no .json files found in {schemas_dir}", file=sys.stderr)
        sys.exit(1)

    return files


def describe_data(data: object) -> str:
    """Inspect rehydrated data for display. Mirrors TS describeData()."""
    if data is None:
        return "null"
    if isinstance(data, list):
        return f"array({len(data)} items)"
    if isinstance(data, dict):
        return f"object({len(data)} keys)"
    return f"{type(data).__name__}: {str(data)[:50]}"


def sanitize_schema_name(name: str) -> str:
    """Sanitize name for OpenAI json_schema.name: ^[a-zA-Z0-9_-]+$ max 64 chars."""
    sanitized = "".join(c if c.isalnum() or c in "_-" else "_" for c in name)
    return sanitized[:64] or "schema"


def test_schema(
    engine: Engine,
    filename: str,
    schemas_dir: str,
    client: OpenAI,
    model: str,
) -> tuple[bool, float]:
    """Run full pipeline for a single schema. Returns (passed, elapsed_seconds)."""
    print(f"\n=== Testing {filename} ===")
    start_time = time.monotonic()

    schema_path = os.path.join(schemas_dir, filename)
    with open(schema_path, encoding="utf-8") as f:
        original_schema = json.load(f)

    try:
        # 1. Convert (WASI wrapper)
        print("  converting...")
        result = engine.convert(
            original_schema,
            {
                "target": "openai-strict",
                "polymorphism": "any-of",
                "max-depth": 50,
                "recursion-limit": 3,
            },
        )

        converted_schema = result["schema"]
        codec = result["codec"]

        # 2. Call OpenAI
        schema_name = "stress_test"

        print(f"  calling {model}...")
        response = client.chat.completions.create(
            model=model,
            messages=[
                {
                    "role": "system",
                    "content": "You are a helpful assistant. Generate valid JSON.",
                },
                {"role": "user", "content": "Generate a complex example."},
            ],
            response_format={
                "type": "json_schema",
                "json_schema": {
                    "name": schema_name,
                    "schema": converted_schema,
                    "strict": True,
                },
            },
        )

        raw_content = response.choices[0].message.content
        if not raw_content:
            raise RuntimeError("No content from OpenAI")

        llm_data = json.loads(raw_content)

        # 3. Rehydrate (WASI wrapper)
        print("  rehydrating...")
        rh_result = engine.rehydrate(llm_data, codec, original_schema)

        if rh_result.get("warnings"):
            print(f"  Warnings: {rh_result['warnings']}")

        rehydrated_data = rh_result["data"]

        # 4. Validate against original schema
        if original_schema is True:
            print("  ✅ Boolean schema (true) — any data valid")
        elif original_schema is False:
            print("  ❌ Boolean schema (false) — no valid data possible")
            elapsed = time.monotonic() - start_time
            return False, elapsed
        elif isinstance(original_schema, dict):
            Draft202012Validator.check_schema(original_schema)
            validator = Draft202012Validator(original_schema)
            validator.validate(rehydrated_data)
            print("  ✅ Validated against original schema")

        # 5. Success
        elapsed = time.monotonic() - start_time
        print(f"  ✅ Success! Rehydrated data: {describe_data(rehydrated_data)}")
        print(f"  ⏱  {elapsed:.2f}s")
        return True, elapsed

    except JslError as e:
        elapsed = time.monotonic() - start_time
        print(f"  ❌ FAIL: {e.message}")
        if e.code:
            print(f"     Code: {e.code}")
        if e.path:
            print(f"     Path: {e.path}")
        print(f"  ⏱  {elapsed:.2f}s")
        return False, elapsed

    except jsonschema.ValidationError as e:
        elapsed = time.monotonic() - start_time
        print(f"  ❌ Validation failed: {e.message}")
        print(f"  ⏱  {elapsed:.2f}s")
        return False, elapsed

    except Exception as e:
        elapsed = time.monotonic() - start_time
        print(f"  ❌ FAIL: {e}")
        print(f"  ⏱  {elapsed:.2f}s")
        return False, elapsed


def main() -> None:
    """Entry point — parse args, run pipeline, report results."""
    args = parse_args()

    schemas_dir = resolve_schemas_dir(args.schemas_dir)
    all_files = load_schema_files(schemas_dir)

    shuffled = fisher_yates_shuffle(all_files, args.seed)

    test_files = shuffled[: min(args.count, len(shuffled))]

    print(
        f"Testing {len(test_files)}/{len(all_files)} schemas "
        f"(model={args.model}, seed={args.seed or 'random'})"
    )

    if not os.environ.get("OPENAI_API_KEY"):
        print("Error: OPENAI_API_KEY environment variable not set.", file=sys.stderr)
        sys.exit(1)

    client = OpenAI()

    passed = 0
    total_time = 0.0
    with Engine() as engine:
        for f in test_files:
            ok, elapsed = test_schema(engine, f, schemas_dir, client, args.model)
            total_time += elapsed
            if ok:
                passed += 1

    print(f"\n\nSummary: {passed}/{len(test_files)} passed ({total_time:.2f}s total).")
    if passed < len(test_files):
        sys.exit(1)


if __name__ == "__main__":
    main()
