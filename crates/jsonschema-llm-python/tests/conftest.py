"""Pytest conftest — shared fixtures for jsonschema-llm Python tests.

Constants and schema loading live in _contract_fixtures.py to avoid
import issues with conftest as a module.
"""

import pytest

from _contract_fixtures import ALL_FIXTURES, FIXTURE_IDS, TARGETS


@pytest.fixture(params=ALL_FIXTURES, ids=FIXTURE_IDS)
def fixture_schema(request):
    """Parametrized fixture yielding (name, schema_dict) tuples."""
    return request.param


@pytest.fixture(params=TARGETS, ids=TARGETS)
def target(request):
    """Parametrized fixture yielding target strings."""
    return request.param
