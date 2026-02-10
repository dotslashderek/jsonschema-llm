"""Acceptance tests for the stress test schema generator.

These tests verify that the generator produces deterministic, diverse,
and correctly-named schema fixtures.
"""

import json
import os
import tempfile
from pathlib import Path

import pytest


@pytest.fixture
def generator_module():
    """Import the generator module dynamically so we can patch its OUTPUT_DIR."""
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "generate_basic_stress",
        Path(__file__).parent.parent / "generate_basic_stress.py",
    )
    mod = importlib.util.module_from_spec(spec)
    return mod, spec


@pytest.fixture
def generate_to_tmpdir(generator_module):
    """Generate schemas into a temp directory and return the path + module."""
    mod, spec = generator_module
    with tempfile.TemporaryDirectory() as tmpdir:
        spec.loader.exec_module(mod)
        mod.OUTPUT_DIR = tmpdir
        mod.main(seed=42)
        yield tmpdir, mod


class TestDeterminism:
    """Finding #4: Same seed must produce identical output."""

    def test_same_seed_produces_same_output(self, generator_module):
        """Two runs with --seed 42 must produce byte-identical fixtures."""
        mod, spec = generator_module
        outputs = []
        for _ in range(2):
            with tempfile.TemporaryDirectory() as tmpdir:
                # Re-import fresh each time
                import importlib.util as iu

                fresh_spec = iu.spec_from_file_location(
                    "generate_basic_stress",
                    Path(__file__).parent.parent / "generate_basic_stress.py",
                )
                fresh_mod = iu.module_from_spec(fresh_spec)
                fresh_spec.loader.exec_module(fresh_mod)
                fresh_mod.OUTPUT_DIR = tmpdir
                fresh_mod.main(seed=42)
                files = {}
                for f in sorted(os.listdir(tmpdir)):
                    if f.endswith(".json"):
                        with open(os.path.join(tmpdir, f)) as fh:
                            files[f] = fh.read()
                outputs.append(files)

        assert outputs[0] == outputs[1], "Generator output is non-deterministic"


class TestVariety:
    """Finding #7: No two fixtures should be structurally identical."""

    def test_no_duplicate_schemas(self, generate_to_tmpdir):
        """Normalized schemas must be unique (no copy-paste duplicates)."""
        tmpdir, _ = generate_to_tmpdir
        schemas = {}
        for f in os.listdir(tmpdir):
            if f.endswith(".json"):
                with open(os.path.join(tmpdir, f)) as fh:
                    content = fh.read()
                    # Normalize: parse and re-serialize with sorted keys
                    normalized = json.dumps(json.loads(content), sort_keys=True)
                    schemas[f] = normalized

        values = list(schemas.values())
        unique = set(values)
        duplicates = len(values) - len(unique)
        assert duplicates == 0, f"Found {duplicates} duplicate schemas"

    def test_minimum_fixture_count(self, generate_to_tmpdir):
        """Generator should produce at least 30 distinct fixtures."""
        tmpdir, _ = generate_to_tmpdir
        fixtures = [f for f in os.listdir(tmpdir) if f.endswith(".json")]
        assert len(fixtures) >= 30, f"Only {len(fixtures)} fixtures generated"


class TestComplexityMetrics:
    """G review: generated fixtures must meet minimum complexity thresholds."""

    def _schema_depth(self, obj, current=0):
        """Calculate max nesting depth of a schema."""
        if not isinstance(obj, dict):
            return current
        max_d = current
        for v in obj.values():
            if isinstance(v, dict):
                max_d = max(max_d, self._schema_depth(v, current + 1))
            elif isinstance(v, list):
                for item in v:
                    if isinstance(item, dict):
                        max_d = max(max_d, self._schema_depth(item, current + 1))
        return max_d

    def _node_count(self, obj):
        """Count total key-value pairs in schema."""
        if not isinstance(obj, dict):
            return 0
        count = len(obj)
        for v in obj.values():
            if isinstance(v, dict):
                count += self._node_count(v)
            elif isinstance(v, list):
                for item in v:
                    if isinstance(item, dict):
                        count += self._node_count(item)
        return count

    def test_has_deep_fixtures(self, generate_to_tmpdir):
        """At least one fixture should have depth >= 10."""
        tmpdir, _ = generate_to_tmpdir
        max_depth = 0
        for f in os.listdir(tmpdir):
            if f.endswith(".json"):
                with open(os.path.join(tmpdir, f)) as fh:
                    try:
                        schema = json.load(fh)
                    except (json.JSONDecodeError, TypeError):
                        continue
                    if isinstance(schema, dict):
                        max_depth = max(max_depth, self._schema_depth(schema))
        assert max_depth >= 10, f"Max depth is only {max_depth}"

    def test_has_wide_fixtures(self, generate_to_tmpdir):
        """At least one fixture should have >= 20 nodes."""
        tmpdir, _ = generate_to_tmpdir
        max_nodes = 0
        for f in os.listdir(tmpdir):
            if f.endswith(".json"):
                with open(os.path.join(tmpdir, f)) as fh:
                    try:
                        schema = json.load(fh)
                    except (json.JSONDecodeError, TypeError):
                        continue
                    if isinstance(schema, dict):
                        max_nodes = max(max_nodes, self._node_count(schema))
        assert max_nodes >= 20, f"Max nodes is only {max_nodes}"


class TestFilenameCorrectness:
    """Finding #14: filenames must match actual schema properties."""

    def test_deep_nesting_filename_matches_depth(self, generate_to_tmpdir):
        """deep_nesting_N.json depth should match N in filename."""
        tmpdir, _ = generate_to_tmpdir
        for f in os.listdir(tmpdir):
            if f.startswith("deep_nesting_") and f.endswith(".json"):
                # Extract depth from filename
                depth_str = f.replace("deep_nesting_", "").replace(".json", "")
                claimed_depth = int(depth_str)
                # Load and measure actual depth
                with open(os.path.join(tmpdir, f)) as fh:
                    schema = json.load(fh)
                actual_depth = 0
                current = schema
                while isinstance(current, dict) and "properties" in current:
                    actual_depth += 1
                    # Follow the first "level_*" property
                    level_keys = [
                        k for k in current["properties"] if k.startswith("level_")
                    ]
                    if level_keys:
                        current = current["properties"][level_keys[0]]
                    else:
                        break
                assert actual_depth == claimed_depth, (
                    f"{f}: claimed depth {claimed_depth}, actual {actual_depth}"
                )


class TestNoUnusedImports:
    """Finding #17: no unused imports in generator script."""

    def test_no_uuid_import(self):
        """The generator should not import uuid (it's unused)."""
        source = (Path(__file__).parent.parent / "generate_basic_stress.py").read_text()
        assert "import uuid" not in source, "Unused 'import uuid' still present"


class TestAllFixturesAreValidJson:
    """All generated fixtures must be parseable JSON."""

    def test_all_json_parseable(self, generate_to_tmpdir):
        tmpdir, _ = generate_to_tmpdir
        for f in os.listdir(tmpdir):
            if f.endswith(".json"):
                with open(os.path.join(tmpdir, f)) as fh:
                    try:
                        json.load(fh)
                    except json.JSONDecodeError as e:
                        pytest.fail(f"{f} is not valid JSON: {e}")
