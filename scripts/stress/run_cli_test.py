"""Run stress tests for jsonschema-llm CLI.

Orchestrates the full pipeline: convert → OpenAI → rehydrate → validate.
Results include per-stage classification with machine-readable reason codes.
"""

import argparse
import json
import os
import re
import subprocess
import time

from openai import OpenAI
import jsonschema


def run_cli_conversion(binary_path, input_path, output_path, codec_path, timeout=30):
    """Convert a JSON Schema to LLM-compatible format.

    Args:
        timeout: Subprocess timeout in seconds (default 30).
    """
    cmd = [
        binary_path,
        "convert",
        input_path,
        "--output",
        output_path,
        "--codec",
        codec_path,
        "--target",
        "openai-strict",
        "--polymorphism",
        "anyof",
    ]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        if result.returncode != 0:
            return False, result.stderr
        return True, result.stderr
    except subprocess.TimeoutExpired:
        return False, f"Timed out after {timeout}s"


def run_cli_rehydration(
    binary_path, input_data_path, codec_path, output_rehydrated_path, schema_path=None, timeout=30
):
    """Rehydrate LLM output using the codec.

    Args:
        schema_path: Optional original schema for type coercion.
        timeout: Subprocess timeout in seconds (default 30).
    """
    cmd = [
        binary_path,
        "rehydrate",
        input_data_path,
        "--codec",
        codec_path,
        "--output",
        output_rehydrated_path,
    ]
    if schema_path:
        cmd.extend(["--schema", schema_path])
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        if result.returncode != 0:
            return False, result.stderr
        return True, result.stderr
    except subprocess.TimeoutExpired:
        return False, f"Timed out after {timeout}s"


def _sanitize_schema_name(name: str) -> str:
    """Sanitize schema name for OpenAI's json_schema.name requirement.

    OpenAI requires: ^[a-zA-Z0-9_-]+$ and max 64 chars.
    """
    sanitized = re.sub(r"[^a-zA-Z0-9_-]", "_", name)
    return sanitized[:64]


def call_openai(client, schema_name, schema_content, model="gpt-4o-mini", timeout=60):
    """Call OpenAI to generate data matching the schema.

    Args:
        model: OpenAI model name (default gpt-4o-mini).
        timeout: API call timeout in seconds (default 60).

    Returns:
        str or None: The response content, or None if content was empty/null.
        Returns error string prefixed with "OPENAI_ERROR:" on exception.
    """
    try:
        completion = client.chat.completions.create(
            model=model,
            messages=[
                {
                    "role": "system",
                    "content": "You are a helpful assistant. Generate a valid JSON object matching the provided schema. Be creative but strict.",
                },
                {"role": "user", "content": "Generate one example."},
            ],
            response_format={
                "type": "json_schema",
                "json_schema": {
                    "name": _sanitize_schema_name(schema_name),
                    "schema": schema_content,
                    "strict": True,
                },
            },
            timeout=timeout,
        )
        content = completion.choices[0].message.content
        # Guard against None content (Finding #1)
        if content is None:
            return None
        return content
    except Exception as e:
        return f"OPENAI_ERROR: {str(e)}"


def validate_original(data, original_schema):
    """Validate rehydrated data against the original schema.

    Uses Draft 2020-12 explicitly so newer keywords (dependentRequired,
    prefixItems, etc.) are not silently ignored.
    """
    try:
        schema = (
            dict(original_schema)
            if isinstance(original_schema, dict)
            else original_schema
        )
        if isinstance(schema, dict) and "$schema" not in schema:
            schema = dict(schema)
            schema["$schema"] = "https://json-schema.org/draft/2020-12/schema"
        validator = jsonschema.Draft202012Validator(schema)
        validator.validate(instance=data)
        return True, ""
    except jsonschema.ValidationError as e:
        return False, str(e)


def main():
    parser = argparse.ArgumentParser(
        description="Run stress tests for jsonschema-llm CLI"
    )
    parser.add_argument("--bin", required=True, help="Path to jsonschema-llm binary")
    parser.add_argument(
        "--schemas", required=True, help="Directory containing input schemas"
    )
    parser.add_argument(
        "--model", default="gpt-4o-mini", help="OpenAI model (default: gpt-4o-mini)"
    )
    parser.add_argument(
        "--timeout-subprocess",
        type=int,
        default=30,
        help="Subprocess timeout in seconds",
    )
    parser.add_argument(
        "--timeout-api", type=int, default=60, help="OpenAI API timeout in seconds"
    )
    parser.add_argument(
        "--seed", type=int, default=None, help="Random seed for reproducibility"
    )
    args = parser.parse_args()

    client = OpenAI()

    schemas = [f for f in os.listdir(args.schemas) if f.endswith(".json")]
    schemas.sort()

    # Apply seed for deterministic schema ordering
    if args.seed is not None:
        import random as _rng

        _rng.seed(args.seed)
        _rng.shuffle(schemas)

    # Run metadata (X review: persist for reproducibility)
    run_metadata = {
        "model": args.model,
        "seed": args.seed,
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "schema_count": len(schemas),
        "timeout_subprocess": args.timeout_subprocess,
        "timeout_api": args.timeout_api,
    }

    results = {"metadata": run_metadata, "pass": [], "fail": []}

    print(f"Starting test run on {len(schemas)} schemas (model={args.model})...")

    output_dir = "stress_results"
    os.makedirs(output_dir, exist_ok=True)

    for schema_file in schemas:
        base_name = os.path.splitext(schema_file)[0]
        input_path = os.path.join(args.schemas, schema_file)
        converted_path = os.path.join(output_dir, f"{base_name}.llm.json")
        codec_path = os.path.join(output_dir, f"{base_name}.codec.json")
        llm_output_path = os.path.join(output_dir, f"{base_name}.openai.json")
        rehydrated_path = os.path.join(output_dir, f"{base_name}.rehydrated.json")

        print(f"Testing {base_name}...", end=" ", flush=True)

        # 1. Convert
        success, err = run_cli_conversion(
            args.bin,
            input_path,
            converted_path,
            codec_path,
            timeout=args.timeout_subprocess,
        )
        if not success:
            print("❌ CONVERT FAIL")
            results["fail"].append(
                {
                    "file": schema_file,
                    "stage": "convert",
                    "reason": "conversion_failed",
                    "error": err,
                }
            )
            continue

        # Load converted schema
        with open(converted_path) as f:
            llm_schema = json.load(f)

        # 2. OpenAI Call
        llm_response_str = call_openai(
            client,
            base_name,
            llm_schema,
            model=args.model,
            timeout=args.timeout_api,
        )

        # Finding #1: Guard against None response
        if llm_response_str is None:
            print("❌ OPENAI FAIL (null content)")
            results["fail"].append(
                {
                    "file": schema_file,
                    "stage": "openai",
                    "reason": "null_content",
                    "error": "OpenAI returned None for message.content",
                }
            )
            continue

        if isinstance(llm_response_str, str) and llm_response_str.startswith(
            "OPENAI_ERROR"
        ):
            print("❌ OPENAI FAIL")
            results["fail"].append(
                {
                    "file": schema_file,
                    "stage": "openai",
                    "reason": "api_error",
                    "error": llm_response_str,
                }
            )
            continue

        # Write LLM response to file for rehydration
        with open(llm_output_path, "w") as f:
            f.write(llm_response_str)

        # 3. Rehydrate
        success, err = run_cli_rehydration(
            args.bin,
            llm_output_path,
            codec_path,
            rehydrated_path,
            schema_path=input_path,
            timeout=args.timeout_subprocess,
        )
        if not success:
            print("❌ REHYDRATE FAIL")
            results["fail"].append(
                {
                    "file": schema_file,
                    "stage": "rehydrate",
                    "reason": "rehydration_failed",
                    "error": err,
                }
            )
            continue

        # 4. Validate rehydrated data against original schema
        with open(rehydrated_path) as f:
            rehydrated_data = json.load(f)
        with open(input_path) as f:
            original_schema = json.load(f)

        valid, err = validate_original(rehydrated_data, original_schema)
        if not valid:
            print("❌ VALIDATION FAIL")
            results["fail"].append(
                {
                    "file": schema_file,
                    "stage": "validation",
                    "reason": "schema_mismatch",
                    "error": err,
                }
            )
            continue

        print("✅ PASS")
        results["pass"].append(schema_file)

    # Summary
    total = len(results["pass"]) + len(results["fail"])
    pass_rate = len(results["pass"]) / total * 100 if total > 0 else 0
    print("\n=== Summary ===")
    print(f"Passed: {len(results['pass'])}/{total} ({pass_rate:.1f}%)")
    print(f"Failed: {len(results['fail'])}/{total}")

    if results["fail"]:
        # Per-stage breakdown
        stages = {}
        for fail in results["fail"]:
            stage = fail["stage"]
            stages[stage] = stages.get(stage, 0) + 1
        print("\nFailures by stage:")
        for stage, count in sorted(stages.items()):
            print(f"  {stage}: {count}")

    report_path = os.path.join(output_dir, "stress_test_report.json")
    with open(report_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nResults written to {report_path}")


if __name__ == "__main__":
    main()
