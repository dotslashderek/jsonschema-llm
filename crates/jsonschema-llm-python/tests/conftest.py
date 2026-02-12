"""Shared pytest fixtures for jsonschema-llm Python contract tests.

Discovers and loads all fixture schemas from the repo-level tests/schemas/
directory tree (top-level, stress/, real-world/).
"""

import json
from pathlib import Path

import pytest

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
# Pytest fixtures
# ---------------------------------------------------------------------------

ALL_FIXTURES = _all_schemas()
FIXTURE_IDS = [name for name, _ in ALL_FIXTURES]


@pytest.fixture(params=ALL_FIXTURES, ids=FIXTURE_IDS)
def fixture_schema(request):
    """Parametrized fixture yielding (name, schema_dict) tuples."""
    return request.param


@pytest.fixture(params=TARGETS, ids=TARGETS)
def target(request):
    """Parametrized fixture yielding target strings."""
    return request.param
