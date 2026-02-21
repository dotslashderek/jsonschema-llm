#!/usr/bin/env python3
"""Pre-process AsyncAPI schema for jsonschema-llm conversion.

The AsyncAPI 2.6 bundled schema contains structures that the converter
can't handle:

1. `examples` arrays with non-schema $ref strings (e.g., #/components/schemas/signup)
2. Meta-schema definitions (Draft-07, OpenAPI 3.0, Avro) that are deeply self-referential
   and cause exponential blowup during ref inlining

This script:
- Strips all `examples` arrays (contain non-schema $ref strings)
- Removes meta-schema definitions that have nested `definitions` blocks
- Replaces any $refs pointing to removed definitions with accept-all `{}`

Usage: python3 preprocess-asyncapi.py <input.json> <output.json>
"""

import json
import sys


def strip_examples(obj):
    """Recursively remove all 'examples' keys from the schema."""
    if isinstance(obj, dict):
        return {k: strip_examples(v) for k, v in obj.items() if k != "examples"}
    elif isinstance(obj, list):
        return [strip_examples(item) for item in obj]
    return obj


def count_self_refs(obj, target_name):
    """Count $ref pointers within a definition that reference the definition itself."""
    count = 0
    if isinstance(obj, dict):
        if "$ref" in obj and isinstance(obj["$ref"], str):
            if f"definitions/{target_name}" in obj["$ref"]:
                count += 1
        for v in obj.values():
            count += count_self_refs(v, target_name)
    elif isinstance(obj, list):
        for item in obj:
            count += count_self_refs(item, target_name)
    return count


def find_problematic_defs(schema):
    """Find definitions that the converter can't handle:
    - Definitions with nested `definitions` blocks (meta-schemas)
    - Definitions with 5+ self-refs (exponential blowup)
    """
    root_defs = schema.get("definitions", {})
    problematic = set()
    for name, defn in root_defs.items():
        if not isinstance(defn, dict):
            continue
        # Meta-schemas with nested definitions
        if isinstance(defn.get("definitions"), dict):
            problematic.add(name)
        # Highly self-referential definitions (e.g., JSON Schema meta-schemas)
        elif count_self_refs(defn, name) >= 5:
            problematic.add(name)
    return problematic


def strip_meta_schemas(schema, meta_names):
    """Remove meta-schema definitions and replace refs pointing to them with {}."""
    root_defs = schema.get("definitions", {})
    for name in meta_names:
        if name in root_defs:
            del root_defs[name]

    # Build set of ref prefixes to replace
    prefixes = tuple(f"#/definitions/{name}" for name in meta_names)

    def rewrite(obj):
        if isinstance(obj, dict):
            if "$ref" in obj and isinstance(obj["$ref"], str):
                if obj["$ref"].startswith(prefixes):
                    # Replace with accept-all schema (remove $ref, return empty object)
                    siblings = {k: v for k, v in obj.items() if k != "$ref"}
                    return rewrite(siblings) if siblings else {}
            return {k: rewrite(v) for k, v in obj.items()}
        elif isinstance(obj, list):
            return [rewrite(item) for item in obj]
        return obj

    return rewrite(schema)


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <input.json> <output.json>", file=sys.stderr)
        sys.exit(1)

    input_path, output_path = sys.argv[1], sys.argv[2]

    with open(input_path) as f:
        schema = json.load(f)

    # Step 1: Strip examples (non-schema $ref strings)
    schema = strip_examples(schema)

    # Step 2: Find and strip problematic definitions
    problematic = find_problematic_defs(schema)
    if problematic:
        print(f"Stripping problematic definitions: {sorted(problematic)}")
        schema = strip_meta_schemas(schema, problematic)

    with open(output_path, "w") as f:
        json.dump(schema, f, indent=2)
        f.write("\n")

    root_defs = schema.get("definitions", {})
    print(f"Output: {len(root_defs)} root definitions")


if __name__ == "__main__":
    main()
