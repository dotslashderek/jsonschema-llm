"""Generate stress test JSON Schema fixtures for jsonschema-llm.

Produces structurally diverse schemas for testing the conversion pipeline.
Domain-specific real-world schemas live in tests/schemas/real-world/ (static).
This generator produces STRUCTURAL stress schemas in tests/schemas/stress/.
"""

import argparse
import json
import os
import random

OUTPUT_DIR = "tests/schemas/stress"


def write_schema(name, schema):
    filename = os.path.join(OUTPUT_DIR, f"{name}.json")
    with open(filename, "w") as f:
        json.dump(schema, f, indent=2)
    print(f"Generated: {filename}")


# --- Generators ---


def gen_deeply_nested(depth=50):
    """Generate a deeply nested object schema.

    The filename explicitly matches the depth argument.
    """
    schema = {"type": "string"}
    for i in range(depth):
        schema = {
            "type": "object",
            "properties": {
                f"level_{depth - i}": schema,
                f"meta_{depth - i}": {"type": "integer"},
            },
            "required": [f"level_{depth - i}"],
        }
    write_schema(f"deep_nesting_{depth}", schema)


def gen_heavy_polymorphism():
    options = []
    options.append({"type": "string", "maxLength": 5})
    options.append({"type": "integer", "multipleOf": 5})
    options.append({"type": "boolean"})
    for i in range(5):
        options.append(
            {
                "type": "object",
                "properties": {
                    "kind": {"const": f"type_{i}"},
                    "value": {"type": "string"},
                },
                "required": ["kind", "value"],
                "additionalProperties": False,
            }
        )
    options.append({"type": "array", "items": {"type": "number"}, "minItems": 10})
    schema = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {"poly_field": {"oneOf": options}},
        "required": ["poly_field"],
    }
    write_schema("heavy_polymorphism_oneof", schema)


def gen_recursive_structures():
    # Linked list
    schema = {
        "$defs": {
            "node": {
                "type": "object",
                "properties": {
                    "value": {"type": "integer"},
                    "next": {"$ref": "#/$defs/node"},
                },
                "required": ["value"],
            }
        },
        "$ref": "#/$defs/node",
    }
    write_schema("recursive_linked_list", schema)

    # Co-recursive (A -> B -> A)
    schema_co = {
        "$defs": {
            "user": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "best_friend": {"$ref": "#/$defs/friend"},
                },
                "required": ["name"],
            },
            "friend": {
                "type": "object",
                "properties": {
                    "nickname": {"type": "string"},
                    "owner": {"$ref": "#/$defs/user"},
                },
            },
        },
        "$ref": "#/$defs/user",
    }
    write_schema("recursive_mutual", schema_co)


def gen_string_constraints():
    patterns = [
        r"^[a-z]+$",
        r"^\d{3}-\d{2}-\d{4}$",
        r"^(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)$",
        r"^([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}$",
    ]
    props = {}
    for i, pat in enumerate(patterns):
        props[f"pattern_{i}"] = {"type": "string", "pattern": pat}
    props["email_format"] = {"type": "string", "format": "email"}
    props["uuid_format"] = {"type": "string", "format": "uuid"}
    schema = {"type": "object", "properties": props, "required": list(props.keys())}
    write_schema("constraints_string_formats", schema)


def gen_numeric_constraints():
    schema = {
        "type": "object",
        "properties": {
            "integer_tiny": {"type": "integer", "minimum": 0, "maximum": 1},
            "number_precise": {
                "type": "number",
                "minimum": 0.0001,
                "maximum": 0.0002,
            },
            "multiple_of_pi": {"type": "number", "multipleOf": 3.14},
            "exclusive_ranges": {
                "type": "number",
                "exclusiveMinimum": 10,
                "exclusiveMaximum": 20,
            },
        },
    }
    write_schema("constraints_numeric_edge_cases", schema)


def gen_array_madness():
    schema = {
        "type": "object",
        "properties": {
            "tuple_fixed": {
                "type": "array",
                "prefixItems": [
                    {"type": "string"},
                    {"type": "integer"},
                    {"type": "boolean"},
                ],
                "items": False,
            },
            "tuple_open": {
                "type": "array",
                "prefixItems": [{"type": "string"}],
                "items": {"type": "integer"},
            },
            "unique_list": {
                "type": "array",
                "items": {"type": "integer"},
                "uniqueItems": True,
            },
            "contains_test": {
                "type": "array",
                "contains": {"const": "magic_token"},
                "minContains": 1,
            },
        },
    }
    write_schema("constraints_array_complexity", schema)


def gen_object_addprops():
    schema = {
        "type": "object",
        "properties": {"builtin": {"type": "string"}},
        "patternProperties": {
            "^S_": {"type": "string"},
            "^I_": {"type": "integer"},
        },
        "additionalProperties": {"type": "boolean"},
    }
    write_schema("object_pattern_additional_props", schema)


def gen_enum_variants():
    schema = {
        "type": "object",
        "properties": {
            "mixed_enum": {"enum": ["red", 1, True, None, {"x": 1}]},
            "const_variant": {"const": "just_this_value"},
        },
    }
    write_schema("enum_heterogeneous", schema)


# --- Genuinely distinct structural generators ---


def gen_recursive_tree():
    """N-ary tree with value, label, and children."""
    schema = {
        "$defs": {
            "tree_node": {
                "type": "object",
                "properties": {
                    "label": {"type": "string"},
                    "value": {"type": "number"},
                    "children": {
                        "type": "array",
                        "items": {"$ref": "#/$defs/tree_node"},
                    },
                },
                "required": ["label"],
            }
        },
        "$ref": "#/$defs/tree_node",
    }
    write_schema("recursive_tree", schema)


def gen_recursive_graph():
    """Graph node with typed edges and adjacency list."""
    schema = {
        "$defs": {
            "graph_node": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "data": {"type": "object", "additionalProperties": True},
                    "edges": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "target": {"$ref": "#/$defs/graph_node"},
                                "weight": {"type": "number"},
                                "label": {"type": "string"},
                            },
                            "required": ["target"],
                        },
                    },
                },
                "required": ["id"],
            }
        },
        "$ref": "#/$defs/graph_node",
    }
    write_schema("recursive_graph", schema)


def gen_recursive_filesystem():
    """Filesystem: files and directories with recursive children."""
    schema = {
        "$defs": {
            "fs_entry": {
                "oneOf": [
                    {
                        "type": "object",
                        "properties": {
                            "type": {"const": "file"},
                            "name": {"type": "string"},
                            "size_bytes": {"type": "integer", "minimum": 0},
                            "permissions": {"type": "string", "pattern": "^[rwx-]{9}$"},
                        },
                        "required": ["type", "name", "size_bytes"],
                    },
                    {
                        "type": "object",
                        "properties": {
                            "type": {"const": "directory"},
                            "name": {"type": "string"},
                            "children": {
                                "type": "array",
                                "items": {"$ref": "#/$defs/fs_entry"},
                            },
                        },
                        "required": ["type", "name", "children"],
                    },
                ]
            }
        },
        "$ref": "#/$defs/fs_entry",
    }
    write_schema("recursive_filesystem", schema)


def gen_compose_allof_deep_merge():
    """allOf with conflicting property constraints requiring deep merge."""
    schema = {
        "allOf": [
            {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "age": {"type": "integer"},
                },
                "required": ["name"],
            },
            {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "maxLength": 100},
                    "email": {"type": "string", "format": "email"},
                },
                "required": ["name", "email"],
            },
        ]
    }
    write_schema("compose_allof_deep_merge", schema)


def gen_compose_nested_oneof():
    """Nested oneOf inside allOf for complex polymorphism."""
    schema = {
        "allOf": [
            {
                "type": "object",
                "properties": {"version": {"type": "integer"}},
                "required": ["version"],
            },
            {
                "oneOf": [
                    {
                        "type": "object",
                        "properties": {
                            "kind": {"const": "text"},
                            "content": {"type": "string"},
                        },
                        "required": ["kind", "content"],
                    },
                    {
                        "type": "object",
                        "properties": {
                            "kind": {"const": "image"},
                            "url": {"type": "string", "format": "uri"},
                            "alt_text": {"type": "string"},
                        },
                        "required": ["kind", "url"],
                    },
                ]
            },
        ]
    }
    write_schema("compose_nested_oneof", schema)


def gen_constraint_dependent_required():
    """dependentRequired: if field A present, field B must also be."""
    schema = {
        "type": "object",
        "properties": {
            "credit_card": {"type": "string"},
            "billing_address": {"type": "string"},
            "shipping_address": {"type": "string"},
        },
        "dependentRequired": {
            "credit_card": ["billing_address"],
        },
    }
    write_schema("constraint_dependent_required", schema)


def gen_constraint_if_then_else():
    """if/then/else conditional schema."""
    schema = {
        "type": "object",
        "properties": {
            "country": {"type": "string"},
            "postal_code": {"type": "string"},
        },
        "required": ["country", "postal_code"],
        "if": {"properties": {"country": {"const": "US"}}},
        "then": {"properties": {"postal_code": {"pattern": "^\\d{5}(-\\d{4})?$"}}},
        "else": {"properties": {"postal_code": {"pattern": "^[A-Z0-9 -]+$"}}},
    }
    write_schema("constraint_if_then_else", schema)


def gen_constraint_pattern_props():
    """patternProperties with additionalProperties: false."""
    schema = {
        "type": "object",
        "patternProperties": {
            "^x-": {"type": "string"},
            "^num_": {"type": "integer", "minimum": 0},
        },
        "additionalProperties": False,
    }
    write_schema("constraint_pattern_props", schema)


def gen_scale_wide_50_props():
    """Wide schema with 50 required string properties."""
    props = {f"field_{i:03d}": {"type": "string"} for i in range(50)}
    schema = {
        "type": "object",
        "properties": props,
        "required": list(props.keys()),
    }
    write_schema("scale_wide_50_props", schema)


def gen_scale_deep_array():
    """Deeply nested array-of-array-of-array structure."""
    schema = {"type": "string"}
    for _ in range(8):
        schema = {"type": "array", "items": schema, "minItems": 1, "maxItems": 3}
    wrapper = {
        "type": "object",
        "properties": {"matrix": schema},
        "required": ["matrix"],
    }
    write_schema("scale_deep_array", wrapper)


def main(seed=42):
    """Generate all stress test schemas.

    Args:
        seed: Random seed for deterministic generation. Default 42.
    """
    random.seed(seed)
    print(f"Generating stress test schemas (seed={seed})...")

    os.makedirs(OUTPUT_DIR, exist_ok=True)

    # Core generators (deterministic)
    gen_deeply_nested(50)
    gen_heavy_polymorphism()
    gen_recursive_structures()
    gen_string_constraints()
    gen_numeric_constraints()
    gen_array_madness()
    gen_object_addprops()
    gen_enum_variants()

    # Genuinely distinct recursive structures
    gen_recursive_tree()
    gen_recursive_graph()
    gen_recursive_filesystem()

    # Composition patterns
    gen_compose_allof_deep_merge()
    gen_compose_nested_oneof()

    # Constraint patterns
    gen_constraint_dependent_required()
    gen_constraint_if_then_else()
    gen_constraint_pattern_props()

    # Scale patterns
    gen_scale_wide_50_props()
    gen_scale_deep_array()

    # --- Combinatorial Generation for Volume ---
    print("Generating combinatorial variations...")

    # Depth & Width variations
    for depth in [5, 10, 50]:
        for width in [2, 5]:
            schema = {"type": "object", "properties": {}, "required": []}
            current = schema
            for d in range(depth):
                prop = f"level_{d}"
                current["properties"][prop] = {
                    "type": "object",
                    "properties": {},
                    "required": [],
                }
                current["required"].append(prop)
                for w in range(width):
                    current["properties"][prop]["properties"][f"item_{w}"] = {
                        "type": "string"
                    }
                current = current["properties"][prop]
            write_schema(f"combo_depth_{depth}_width_{width}", schema)

    # Array permutations (deterministic via seed)
    types = ["string", "integer", "boolean", "null"]
    for i in range(10):
        prefix = [{"type": random.choice(types)} for _ in range(random.randint(1, 5))]
        schema = {
            "type": "array",
            "prefixItems": prefix,
            "items": {"type": "string"},
            "minItems": len(prefix),
            "maxItems": len(prefix) + 5,
            "description": f"Tuple variant {i}",
        }
        write_schema(f"combo_array_tuple_{i}", schema)

    # Polymorphism mixtures (oneOf nested in allOf)
    for i in range(10):
        schema = {
            "allOf": [
                {
                    "type": "object",
                    "properties": {"common": {"const": f"val_{i}"}},
                    "required": ["common"],
                },
                {
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {"a": {"type": "string"}},
                        },
                        {
                            "type": "object",
                            "properties": {"b": {"type": "integer"}},
                        },
                    ]
                },
            ]
        }
        write_schema(f"combo_poly_mix_{i}", schema)

    # Edge Cases
    write_schema("edge_true", True)
    write_schema("edge_false", False)
    write_schema("edge_empty", {})
    write_schema("edge_not_string", {"not": {"type": "string"}})

    # Dangerous Property Names
    reserved = [
        "class",
        "int",
        "float",
        "return",
        "function",
        "var",
        "let",
        "const",
        "try",
        "catch",
    ]
    schema = {
        "type": "object",
        "properties": {k: {"type": "string"} for k in reserved},
        "required": reserved,
    }
    write_schema("edge_reserved_words", schema)

    # Unicode Madness
    unicode_keys = ["🚀", "你好", "☃️", "a list"]
    schema = {
        "type": "object",
        "properties": {k: {"type": "string"} for k in unicode_keys},
        "required": unicode_keys,
    }
    write_schema("edge_unicode_keys", schema)

    print(f"Done. Total schemas in {OUTPUT_DIR}: {len(os.listdir(OUTPUT_DIR))}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Generate stress test schemas")
    parser.add_argument(
        "--seed", type=int, default=42, help="Random seed (default: 42)"
    )
    parser.add_argument(
        "--clean",
        action="store_true",
        help="Remove existing schemas from output directory before generating",
    )
    args = parser.parse_args()
    if args.clean and os.path.isdir(OUTPUT_DIR):
        for f in os.listdir(OUTPUT_DIR):
            if f.endswith(".json"):
                os.remove(os.path.join(OUTPUT_DIR, f))
        print(f"Cleaned {OUTPUT_DIR}")
    main(seed=args.seed)
