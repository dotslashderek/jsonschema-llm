"""Run stress tests for jsonschema-llm CLI.

Orchestrates the full pipeline: convert → OpenAI → rehydrate → validate.
Results include per-stage classification with machine-readable reason codes.
"""

import argparse
import json
import os
import random
import re
import subprocess
import sys
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
            return False, result.stderr, False
        return True, result.stderr, False
    except subprocess.TimeoutExpired:
        return False, f"Timed out after {timeout}s", True


def run_cli_rehydration(
    binary_path,
    input_data_path,
    codec_path,
    output_rehydrated_path,
    schema_path=None,
    timeout=30,
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
            return False, result.stderr, False
        return True, result.stderr, False
    except subprocess.TimeoutExpired:
        return False, f"Timed out after {timeout}s", True


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


def _is_transient_failure(attempt):
    """Check whether a failure is transient (worth retrying).

    Transient reasons: api_error, null_content, timeout.
    Permanent reasons: conversion_failed, rehydration_failed, schema_mismatch.
    """
    return attempt.get("reason") in ("api_error", "null_content", "timeout")


def run_single_schema(
    binary_path,
    schema_file,
    schemas_dir,
    output_dir,
    client,
    model="gpt-4o-mini",
    timeout_subprocess=30,
    timeout_api=60,
    retries=0,
    retry_delay=2,
    max_delay=60,
):
    """Run the full pipeline for a single schema, with optional retries.

    Uses exponential backoff with jitter for transient failures.
    Permanent failures (conversion_failed, rehydration_failed, schema_mismatch)
    skip retries entirely.

    Args:
        retries: Number of additional attempts on failure (0 = no retries).
        retry_delay: Base delay in seconds for exponential backoff (default 2).
        max_delay: Maximum delay cap in seconds (default 60).

    Returns:
        dict with keys: file, verdict, attempts.
        verdict is one of: solid_pass, flaky_pass, solid_fail.
        attempts is a list of dicts with: passed, stage, reason, error.
    """
    base_name = os.path.splitext(schema_file)[0]
    input_path = os.path.join(schemas_dir, schema_file)
    converted_path = os.path.join(output_dir, f"{base_name}.llm.json")
    codec_path = os.path.join(output_dir, f"{base_name}.codec.json")
    llm_output_path = os.path.join(output_dir, f"{base_name}.openai.json")
    rehydrated_path = os.path.join(output_dir, f"{base_name}.rehydrated.json")

    max_attempts = 1 + retries
    attempts = []

    for attempt_num in range(max_attempts):
        if attempt_num > 0:
            base = retry_delay * (2 ** (attempt_num - 1))
            capped = min(max_delay, base)
            delay = random.uniform(capped * 0.75, capped * 1.25)
            delay = min(delay, max_delay)
            time.sleep(delay)

        attempt = _run_pipeline_once(
            binary_path,
            input_path,
            converted_path,
            codec_path,
            llm_output_path,
            rehydrated_path,
            base_name,
            client,
            model,
            timeout_subprocess,
            timeout_api,
        )
        attempts.append(attempt)

        if attempt["passed"]:
            break

        # Skip retries for permanent failures
        if not _is_transient_failure(attempt):
            break

    # Determine verdict
    any_passed = any(a["passed"] for a in attempts)
    if any_passed:
        if attempts[0]["passed"]:
            verdict = "solid_pass"
        else:
            verdict = "flaky_pass"
    else:
        verdict = "solid_fail"

    return {"file": schema_file, "verdict": verdict, "attempts": attempts}


def _run_pipeline_once(
    binary_path,
    input_path,
    converted_path,
    codec_path,
    llm_output_path,
    rehydrated_path,
    base_name,
    client,
    model,
    timeout_subprocess,
    timeout_api,
):
    """Execute one attempt of the convert→openai→rehydrate→validate pipeline.

    Returns:
        dict with keys: passed, stage, reason, error.
    """
    # 1. Convert
    success, err, is_timeout = run_cli_conversion(
        binary_path,
        input_path,
        converted_path,
        codec_path,
        timeout=timeout_subprocess,
    )
    if not success:
        return {
            "passed": False,
            "stage": "convert",
            "reason": "timeout" if is_timeout else "conversion_failed",
            "error": err,
        }

    # Load converted schema
    with open(converted_path) as f:
        llm_schema = json.load(f)

    # 2. OpenAI Call
    llm_response_str = call_openai(
        client, base_name, llm_schema, model=model, timeout=timeout_api
    )

    if llm_response_str is None:
        return {
            "passed": False,
            "stage": "openai",
            "reason": "null_content",
            "error": "OpenAI returned None for message.content",
        }

    if isinstance(llm_response_str, str) and llm_response_str.startswith(
        "OPENAI_ERROR"
    ):
        return {
            "passed": False,
            "stage": "openai",
            "reason": "api_error",
            "error": llm_response_str,
        }

    # Write LLM response to file for rehydration
    with open(llm_output_path, "w") as f:
        f.write(llm_response_str)

    # 3. Rehydrate
    success, err, is_timeout = run_cli_rehydration(
        binary_path,
        llm_output_path,
        codec_path,
        rehydrated_path,
        schema_path=input_path,
        timeout=timeout_subprocess,
    )
    if not success:
        return {
            "passed": False,
            "stage": "rehydrate",
            "reason": "timeout" if is_timeout else "rehydration_failed",
            "error": err,
        }

    # 4. Validate rehydrated data against original schema
    with open(rehydrated_path) as f:
        rehydrated_data = json.load(f)
    with open(input_path) as f:
        original_schema = json.load(f)

    valid, err = validate_original(rehydrated_data, original_schema)
    if not valid:
        return {
            "passed": False,
            "stage": "validation",
            "reason": "schema_mismatch",
            "error": err,
        }

    return {"passed": True, "stage": None, "reason": None, "error": ""}


def load_expected_failures(config_path):
    """Load expected-failures configuration from a JSON file.

    Args:
        config_path: Path to the JSON config file.

    Returns:
        dict mapping schema base names to their config (reason, optional stage).

    Raises:
        SystemExit: If file not found or invalid JSON.
    """
    try:
        with open(config_path) as f:
            config = json.load(f)
    except FileNotFoundError:
        print(
            f"Error: expected-failures config not found: {config_path}", file=sys.stderr
        )
        sys.exit(2)
    except json.JSONDecodeError as e:
        print(
            f"Error: invalid JSON in expected-failures config {config_path}: {e}",
            file=sys.stderr,
        )
        sys.exit(2)
    if not isinstance(config, dict):
        print(
            f"Error: top-level config in {config_path} must be an object, got {type(config).__name__}",
            file=sys.stderr,
        )
        sys.exit(2)
    schemas = config.get("schemas", {})
    if not isinstance(schemas, dict):
        print(
            f"Error: 'schemas' in {config_path} must be an object, got {type(schemas).__name__}",
            file=sys.stderr,
        )
        sys.exit(2)
    for name, entry in schemas.items():
        if not isinstance(entry, dict):
            print(
                f"Error: schemas['{name}'] in {config_path} must be an object, got {type(entry).__name__}",
                file=sys.stderr,
            )
            sys.exit(2)
    return schemas


def classify_result(result, expected_failures):
    """Classify a schema result considering the expected-failures config.

    Args:
        result: dict with file, verdict, attempts.
        expected_failures: dict from load_expected_failures, or empty dict.

    Returns:
        One of: solid_pass, flaky_pass, solid_fail, expected_fail, unexpected_pass.
    """
    base_name = os.path.splitext(result["file"])[0]

    if base_name not in expected_failures:
        return result["verdict"]

    config = expected_failures[base_name]
    is_pass = result["verdict"] in ("solid_pass", "flaky_pass")

    if is_pass:
        return "unexpected_pass"

    # It failed — check if stage matches
    expected_stage = config.get("stage")
    if expected_stage is not None:
        # Stage-aware: only count as expected if failure stage matches
        last_attempt = result["attempts"][-1]
        actual_stage = last_attempt.get("stage")
        if actual_stage != expected_stage:
            return "solid_fail"

    return "expected_fail"


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
    parser.add_argument(
        "--retries",
        type=int,
        default=0,
        help="Number of retries on failure (default: 0, recommended: 2)",
    )
    parser.add_argument(
        "--retry-delay",
        type=float,
        default=2,
        help="Base delay in seconds for exponential backoff (default: 2)",
    )
    parser.add_argument(
        "--max-delay",
        type=float,
        default=60,
        help="Maximum delay cap in seconds (default: 60)",
    )
    parser.add_argument(
        "--expected-failures",
        default=None,
        help="Path to expected-failures JSON config",
    )
    args = parser.parse_args()

    if args.retries < 0:
        parser.error("--retries must be >= 0")

    client = OpenAI()

    schemas = [f for f in os.listdir(args.schemas) if f.endswith(".json")]
    schemas.sort()

    # Apply seed for deterministic schema ordering
    if args.seed is not None:
        import random as _rng

        _rng.seed(args.seed)
        _rng.shuffle(schemas)

    # Load expected failures config
    expected_failures = {}
    if args.expected_failures:
        ef_path = args.expected_failures
        # Resolve relative to script location if not absolute
        if not os.path.isabs(ef_path):
            script_dir = os.path.dirname(os.path.abspath(__file__))
            ef_path = os.path.join(script_dir, ef_path)
        expected_failures = load_expected_failures(ef_path)

    # Run metadata (X review: persist for reproducibility)
    run_metadata = {
        "model": args.model,
        "seed": args.seed,
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "schema_count": len(schemas),
        "timeout_subprocess": args.timeout_subprocess,
        "timeout_api": args.timeout_api,
        "retries": args.retries,
        "retry_delay": args.retry_delay,
        "max_delay": args.max_delay,
        "expected_failures_config": ef_path if args.expected_failures else None,
    }

    # Backward-compat result structure + new detailed_results
    results = {
        "metadata": run_metadata,
        "pass": [],
        "fail": [],
        "detailed_results": [],
    }

    # Classification counters
    solid_passes = []
    flaky_passes = []
    solid_fails = []
    expected_fail_list = []
    unexpected_pass_list = []

    retries_label = f", retries={args.retries}" if args.retries > 0 else ""
    print(
        f"Starting test run on {len(schemas)} schemas (model={args.model}{retries_label})..."
    )

    output_dir = "stress_results"
    os.makedirs(output_dir, exist_ok=True)

    for schema_file in schemas:
        base_name = os.path.splitext(schema_file)[0]
        max_attempts = 1 + args.retries

        print(f"Testing {base_name}...", end=" ", flush=True)

        result = run_single_schema(
            binary_path=args.bin,
            schema_file=schema_file,
            schemas_dir=args.schemas,
            output_dir=output_dir,
            client=client,
            model=args.model,
            timeout_subprocess=args.timeout_subprocess,
            timeout_api=args.timeout_api,
            retries=args.retries,
            retry_delay=args.retry_delay,
            max_delay=args.max_delay,
        )

        # Print per-attempt progress for retries
        if args.retries > 0 and len(result["attempts"]) > 1:
            for i, attempt in enumerate(result["attempts"]):
                attempt_label = f"({i + 1}/{max_attempts})"
                separator = " → " if i < len(result["attempts"]) - 1 else ""
                if attempt["passed"]:
                    print(f"✅ PASS {attempt_label}{separator}", end="", flush=True)
                else:
                    print(f"❌ FAIL {attempt_label}{separator}", end="", flush=True)
            print(" ", end="")

        # Classify with expected failures
        classification = classify_result(result, expected_failures)

        # Console output
        if classification == "solid_pass":
            print("✅ PASS")
            results["pass"].append(schema_file)
            solid_passes.append(result)
        elif classification == "flaky_pass":
            print("Final: ⚠️ PASS (flaky)")
            results["pass"].append(schema_file)
            flaky_passes.append(result)
        elif classification == "expected_fail":
            reason = expected_failures.get(base_name, {}).get("reason", "unknown")
            print(f"🔇 EXPECTED FAIL ({reason})")
            results["fail"].append(
                {
                    "file": schema_file,
                    "stage": result["attempts"][-1].get("stage", "unknown"),
                    "reason": "expected_fail",
                    "error": reason,
                }
            )
            expected_fail_list.append(result)
        elif classification == "unexpected_pass":
            print("🚨 UNEXPECTED PASS")
            results["pass"].append(schema_file)
            unexpected_pass_list.append(result)
        else:
            # solid_fail
            last_attempt = result["attempts"][-1]
            stage = last_attempt.get("stage", "unknown")
            print(f"❌ {stage.upper()} FAIL")
            results["fail"].append(
                {
                    "file": schema_file,
                    "stage": stage,
                    "reason": last_attempt.get("reason", "unknown"),
                    "error": last_attempt.get("error", ""),
                }
            )
            solid_fails.append(result)

        # Add to detailed results
        result["classification"] = classification
        results["detailed_results"].append(result)

    # Summary
    tested = len(schemas) - len(expected_fail_list)
    passed = len(solid_passes) + len(flaky_passes) + len(unexpected_pass_list)
    pass_rate = passed / tested * 100 if tested > 0 else 0

    print("\n=== Summary ===")
    print(f"Passed: {passed}/{tested} ({pass_rate:.1f}%)", end="")
    if expected_fail_list:
        print(f" | Expected Failures: {len(expected_fail_list)}", end="")
    if unexpected_pass_list:
        print(f" | Unexpected Passes: {len(unexpected_pass_list)}", end="")
    print()

    if flaky_passes:
        print(f"  ⚠️  Flaky passes: {len(flaky_passes)}")
        for r in flaky_passes:
            print(
                f"    {os.path.splitext(r['file'])[0]} ({len(r['attempts'])} attempts)"
            )

    if solid_fails:
        print(f"  ❌ Solid failures: {len(solid_fails)}")
        # Per-stage breakdown
        stages = {}
        for r in solid_fails:
            stage = r["attempts"][-1].get("stage", "unknown")
            stages[stage] = stages.get(stage, 0) + 1
        print("  Failures by stage:")
        for stage, count in sorted(stages.items()):
            print(f"    {stage}: {count}")

    if expected_fail_list:
        print(f"  🔇 Expected failures: {len(expected_fail_list)}")
        for r in expected_fail_list:
            name = os.path.splitext(r["file"])[0]
            reason = expected_failures.get(name, {}).get("reason", "unknown")
            print(f"    {name}: {reason}")

    if unexpected_pass_list:
        print(f"  🚨 Unexpected passes: {len(unexpected_pass_list)}")
        for r in unexpected_pass_list:
            print(f"    {os.path.splitext(r['file'])[0]}")

    report_path = os.path.join(output_dir, "stress_test_report.json")
    with open(report_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nResults written to {report_path}")

    # Exit code: fail on solid_fail or unexpected_pass
    if solid_fails or unexpected_pass_list:
        sys.exit(1)


if __name__ == "__main__":
    main()
