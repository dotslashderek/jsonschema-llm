"""Pytest conftest — shared fixtures for jsonschema-llm Python tests.

Constants and schema loading live in _contract_fixtures.py to avoid
import issues with conftest as a module.
"""

import pytest

from _contract_fixtures import ALL_FIXTURES, FIXTURE_IDS, TARGETS, load_kitchen_sink


def pytest_collection_modifyitems(config, items):
    """Fail fast if no fixture schemas were discovered."""
    if not ALL_FIXTURES:
        pytest.exit(
            "No fixture schemas found — check that tests/schemas/ exists "
            "relative to the repo root.",
            returncode=1,
        )


@pytest.fixture(params=ALL_FIXTURES, ids=FIXTURE_IDS)
def fixture_schema(request):
    """Parametrized fixture yielding (name, schema_dict) tuples."""
    return request.param


@pytest.fixture(params=TARGETS, ids=TARGETS)
def target(request):
    """Parametrized fixture yielding target strings."""
    return request.param


@pytest.fixture()
def kitchen_sink_schema():
    """Shared fixture for kitchen_sink.json, skips if missing."""
    schema = load_kitchen_sink()
    if schema is None:
        pytest.skip("kitchen_sink.json not found")
    return schema
