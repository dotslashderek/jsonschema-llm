"""Shared constants and schema loading for contract tests.

Separated from conftest.py so test modules can import without
relying on pytest's conftest plugin loading mechanism.
"""

import json
from pathlib import Path

# ---------------------------------------------------------------------------
# Path anchors
# ---------------------------------------------------------------------------

# crates/jsonschema-llm-python/tests/ → repo root
REPO_ROOT = Path(__file__).resolve().parents[3]
SCHEMAS_DIR = REPO_ROOT / "tests" / "schemas"
SNAPSHOTS_DIR = REPO_ROOT / "tests" / "snapshots"

TARGETS = ["openai-strict", "gemini", "claude"]

# Error codes that are expected during parametrized testing and should
# be treated as xfail/skip rather than hard failures.
EXPECTED_CONVERT_ERRORS = frozenset({"recursion_depth_exceeded"})
EXPECTED_REHYDRATE_ERRORS = frozenset({"rehydration_error"})


# ---------------------------------------------------------------------------
# Schema loading helpers
# ---------------------------------------------------------------------------


def _load_schemas_from(directory: Path) -> list[tuple[str, dict]]:
    """Load all .json files from a directory as (name, schema) tuples."""
    if not directory.is_dir():
        return []
    schemas = []
    for path in sorted(directory.glob("*.json")):
        with open(path, encoding="utf-8") as f:
            schemas.append((path.stem, json.load(f)))
    return schemas


def _all_schemas() -> list[tuple[str, dict]]:
    """Load schemas from all three fixture directories."""
    schemas = []
    schemas.extend(_load_schemas_from(SCHEMAS_DIR))
    schemas.extend(_load_schemas_from(SCHEMAS_DIR / "stress"))
    schemas.extend(_load_schemas_from(SCHEMAS_DIR / "real-world"))
    return schemas


def load_kitchen_sink() -> dict | None:
    """Load kitchen_sink.json, returning None if not found."""
    path = SCHEMAS_DIR / "kitchen_sink.json"
    if not path.exists():
        return None
    with open(path, encoding="utf-8") as f:
        return json.load(f)


# ---------------------------------------------------------------------------
# Pre-loaded fixtures
# ---------------------------------------------------------------------------

ALL_FIXTURES = _all_schemas()
FIXTURE_IDS = [name for name, _ in ALL_FIXTURES]
