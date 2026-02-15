"""Compare two stress-test reports for regression tracking.

Usage:
    python compare_reports.py baseline.json current.json [--json] [--strict]
"""

import argparse
import json
import os
import sys
from dataclasses import asdict, dataclass, field


@dataclass
class ComparisonResult:
    """Result of diffing two stress-test reports."""

    new_passes: list = field(default_factory=list)
    new_failures: list = field(default_factory=list)
    fixes: list = field(default_factory=list)
    new_flaky: list = field(default_factory=list)
    config_drift: list = field(default_factory=list)
    unchanged: list = field(default_factory=list)
    baseline_only: list = field(default_factory=list)
    current_only: list = field(default_factory=list)
    baseline_pass_rate: float = 0.0
    current_pass_rate: float = 0.0


def load_report(path):
    """Load and validate a stress-test report JSON file.

    Args:
        path: Path to the JSON report.

    Returns:
        Parsed report dict.

    Raises:
        FileNotFoundError: If the file doesn't exist.
        SystemExit: If the JSON is invalid.
    """
    if not os.path.exists(path):
        raise FileNotFoundError(f"Report not found: {path}")
    try:
        with open(path) as f:
            return json.load(f)
    except json.JSONDecodeError as e:
        print(f"Error: invalid JSON in {path}: {e}", file=sys.stderr)
        sys.exit(2)


def _extract_verdicts(report):
    """Extract schema base_name → classification from detailed_results.

    Falls back to verdict if classification not present.
    """
    verdicts = {}
    for entry in report.get("detailed_results", []):
        base_name = os.path.splitext(entry["file"])[0]
        verdicts[base_name] = entry.get("classification", entry.get("verdict"))
    return verdicts


_PASSING = frozenset({"solid_pass", "flaky_pass", "unexpected_pass"})


def _pass_rate(verdicts):
    """Compute pass rate as a percentage."""
    if not verdicts:
        return 0.0
    passing = sum(1 for v in verdicts.values() if v in _PASSING)
    return (passing / len(verdicts)) * 100.0


def compare_reports(baseline, current):
    """Diff two reports and categorize schema transitions.

    Args:
        baseline: Parsed baseline report dict.
        current: Parsed current report dict.

    Returns:
        ComparisonResult with categorized transitions.
    """
    base_verdicts = _extract_verdicts(baseline)
    curr_verdicts = _extract_verdicts(current)

    base_keys = set(base_verdicts)
    curr_keys = set(curr_verdicts)

    result = ComparisonResult(
        baseline_pass_rate=_pass_rate(base_verdicts),
        current_pass_rate=_pass_rate(curr_verdicts),
        baseline_only=sorted(base_keys - curr_keys),
        current_only=sorted(curr_keys - base_keys),
    )

    common = base_keys & curr_keys
    for schema in sorted(common):
        old = base_verdicts[schema]
        new = curr_verdicts[schema]

        if old == new:
            result.unchanged.append(schema)
            continue

        # Fixes: expected_fail → any pass
        if old == "expected_fail" and new in _PASSING:
            result.fixes.append(schema)
        # Config drift: unexpected_pass → solid_pass (config needs updating)
        elif old == "unexpected_pass" and new == "solid_pass":
            result.config_drift.append(schema)
        # New flaky: stable pass → flaky
        elif old == "solid_pass" and new == "flaky_pass":
            result.new_flaky.append(schema)
        # New failures: was passing → now failing
        elif old in _PASSING and new not in _PASSING:
            result.new_failures.append(schema)
        # New passes: was failing → now passing
        elif old not in _PASSING and new in _PASSING:
            result.new_passes.append(schema)
        else:
            # Other transitions (e.g., solid_fail → expected_fail)
            result.unchanged.append(schema)

    return result


def get_exit_code(result, strict=False):
    """Determine exit code based on regressions.

    Args:
        result: ComparisonResult.
        strict: If True, new flakiness also causes exit 1.

    Returns:
        0 if clean, 1 if regressions detected.
    """
    if result.new_failures:
        return 1
    if strict and result.new_flaky:
        return 1
    return 0


def format_comparison(result, json_output=False):
    """Format comparison result as human-readable text or JSON.

    Args:
        result: ComparisonResult.
        json_output: If True, return JSON string.

    Returns:
        Formatted string.
    """
    if json_output:
        return json.dumps(asdict(result), indent=2)

    lines = ["=== Stress Report Comparison ===", ""]

    if result.new_failures:
        lines.append(f"❌ New failures ({len(result.new_failures)}):")
        for s in result.new_failures:
            lines.append(f"  - {s}")
        lines.append("")

    if result.new_flaky:
        lines.append(f"⚠️  New flaky ({len(result.new_flaky)}):")
        for s in result.new_flaky:
            lines.append(f"  - {s}")
        lines.append("")

    if result.new_passes:
        lines.append(f"✅ New passes ({len(result.new_passes)}):")
        for s in result.new_passes:
            lines.append(f"  - {s}")
        lines.append("")

    if result.fixes:
        lines.append(f"🔧 Fixes ({len(result.fixes)}):")
        for s in result.fixes:
            lines.append(f"  - {s}")
        lines.append("")

    if result.config_drift:
        lines.append(f"📋 Config drift ({len(result.config_drift)}):")
        for s in result.config_drift:
            lines.append(f"  - {s}")
        lines.append("")

    if result.baseline_only:
        lines.append(f"🗑️  Removed ({len(result.baseline_only)}):")
        for s in result.baseline_only:
            lines.append(f"  - {s}")
        lines.append("")

    if result.current_only:
        lines.append(f"🆕 Added ({len(result.current_only)}):")
        for s in result.current_only:
            lines.append(f"  - {s}")
        lines.append("")

    lines.append(f"Unchanged: {len(result.unchanged)} schemas")
    lines.append(
        f"Pass rate: {result.baseline_pass_rate:.1f}% → {result.current_pass_rate:.1f}%"
    )

    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(
        description="Compare two stress-test reports for regressions"
    )
    parser.add_argument("baseline", help="Path to baseline report JSON")
    parser.add_argument("current", help="Path to current report JSON")
    parser.add_argument("--json", action="store_true", help="Output comparison as JSON")
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Exit 1 on new flakiness (for CI)",
    )
    args = parser.parse_args()

    baseline = load_report(args.baseline)
    current = load_report(args.current)

    result = compare_reports(baseline, current)
    print(format_comparison(result, json_output=args.json))

    sys.exit(get_exit_code(result, strict=args.strict))


if __name__ == "__main__":
    main()
