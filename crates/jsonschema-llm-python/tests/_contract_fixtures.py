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


# ---------------------------------------------------------------------------
# Schema loading helpers
# ---------------------------------------------------------------------------


def _load_schemas_from(directory: Path) -> list[tuple[str, dict]]:
    """Load all .json files from a directory as (name, schema) tuples."""
    if not directory.is_dir():
        return []
    schemas = []
    for path in sorted(directory.glob("*.json")):
        with open(path) as f:
            schemas.append((path.stem, json.load(f)))
    return schemas


def _all_schemas() -> list[tuple[str, dict]]:
    """Load schemas from all three fixture directories."""
    schemas = []
    schemas.extend(_load_schemas_from(SCHEMAS_DIR))
    schemas.extend(_load_schemas_from(SCHEMAS_DIR / "stress"))
    schemas.extend(_load_schemas_from(SCHEMAS_DIR / "real-world"))
    return schemas


# ---------------------------------------------------------------------------
# Pre-loaded fixtures
# ---------------------------------------------------------------------------

ALL_FIXTURES = _all_schemas()
assert ALL_FIXTURES, (
    f"No fixture schemas found — expected schemas in {SCHEMAS_DIR}. "
    "Check that the repo root is correctly resolved."
)
FIXTURE_IDS = [name for name, _ in ALL_FIXTURES]
