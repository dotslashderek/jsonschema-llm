import json
import os
import random
import uuid

OUTPUT_DIR = "tests/schemas/stress"
os.makedirs(OUTPUT_DIR, exist_ok=True)

def write_schema(name, schema):
    filename = os.path.join(OUTPUT_DIR, f"{name}.json")
    with open(filename, "w") as f:
        json.dump(schema, f, indent=2)
    print(f"Generated: {filename}")

# --- Generators ---

def gen_deeply_nested(depth=20):
    schema = {"type": "string"}
    for i in range(depth):
        schema = {
            "type": "object",
            "properties": {
                f"level_{depth-i}": schema,
                f"meta_{depth-i}": {"type": "integer"}
            },
            "required": [f"level_{depth-i}"]
        }
    write_schema("deep_nesting_20", schema)

def gen_heavy_polymorphism():
    # Massive oneOf over diverse types
    options = []
    # 1. Primitives
    options.append({"type": "string", "maxLength": 5})
    options.append({"type": "integer", "multipleOf": 5})
    options.append({"type": "boolean"})
    # 2. Objects
    for i in range(5):
        options.append({
            "type": "object",
            "properties": {
                "kind": {"const": f"type_{i}"},
                "value": {"type": "string"}
            },
            "required": ["kind", "value"],
            "additionalProperties": False
        })
    # 3. Arrays
    options.append({
        "type": "array",
        "items": {"type": "number"},
        "minItems": 10
    })
    
    schema = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "poly_field": {"oneOf": options}
        },
        "required": ["poly_field"]
    }
    write_schema("heavy_polymorphism_oneof", schema)

def gen_recursive_structures():
    # Linked List
    schema = {
        "$defs": {
            "node": {
                "type": "object",
                "properties": {
                    "value": {"type": "integer"},
                    "next": {"$ref": "#/$defs/node"}
                },
                "required": ["value"]
            }
        },
        "$ref": "#/$defs/node"
    }
    write_schema("recursive_linked_list", schema)

    # Co-recursive (A -> B -> A)
    schema_co = {
        "$defs": {
            "user": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "best_friend": {"$ref": "#/$defs/friend"}
                },
                "required": ["name"]
            },
            "friend": {
                "type": "object",
                "properties": {
                    "nickname": {"type": "string"},
                    "owner": {"$ref": "#/$defs/user"}
                }
            }
        },
        "$ref": "#/$defs/user"
    }
    write_schema("recursive_mutual", schema_co)

def gen_string_constraints():
    # Regex madness
    patterns = [
        r"^[a-z]+$", 
        r"^\d{3}-\d{2}-\d{4}$", # SSN-ish
        r"^(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)$", # IPv4
        r"^([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}$" # IPv6 simple
    ]
    
    props = {}
    for i, pat in enumerate(patterns):
        props[f"pattern_{i}"] = {"type": "string", "pattern": pat}
    
    props["email_format"] = {"type": "string", "format": "email"}
    props["uuid_format"] = {"type": "string", "format": "uuid"}
    # datetime with bounds? (not standard json schema but useful to test handling)
    
    schema = {
        "type": "object",
        "properties": props,
        "required": list(props.keys())
    }
    write_schema("constraints_string_formats", schema)

def gen_numeric_constraints():
    # Weird number ranges
    schema = {
        "type": "object",
        "properties": {
            "integer_tiny": {"type": "integer", "minimum": 0, "maximum": 1},
            "number_precise": {"type": "number", "minimum": 0.0001, "maximum": 0.0002},
            "multiple_of_pi": {"type": "number", "multipleOf": 3.14}, # Approximate
            "exclusive_ranges": {
                "type": "number", 
                "exclusiveMinimum": 10, 
                "exclusiveMaximum": 20
            }
        }
    }
    write_schema("constraints_numeric_edge_cases", schema)

def gen_array_madness():
    # Tuples, uniqueItems, contains
    schema = {
        "type": "object",
        "properties": {
            "tuple_fixed": {
                "type": "array",
                "prefixItems": [
                    {"type": "string"},
                    {"type": "integer"},
                    {"type": "boolean"}
                ],
                "items": False # No additional items
            },
            "tuple_open": {
                "type": "array",
                "prefixItems": [{"type": "string"}],
                "items": {"type": "integer"} # Rest are ints
            },
            "unique_list": {
                "type": "array",
                "items": {"type": "integer"},
                "uniqueItems": True
            },
            "contains_test": {
                "type": "array",
                "contains": {"const": "magic_token"},
                "minContains": 1
            }
        }
    }
    write_schema("constraints_array_complexity", schema)

def gen_object_addprops():
    # testing additionalProperties interaction
    schema = {
        "type": "object",
        "properties": {
            "builtin": {"type": "string"}
        },
        "patternProperties": {
            "^S_": {"type": "string"},
            "^I_": {"type": "integer"}
        },
        "additionalProperties": {"type": "boolean"} 
    }
    write_schema("object_pattern_additional_props", schema)

def gen_enum_variants():
    # Heterogeneous enums
    schema = {
        "type": "object",
        "properties": {
            "mixed_enum": {
                "enum": ["red", 1, True, None, {"x": 1}] # "enum" accepts any JSON value
            },
            "const_variant": {
                "const": "just_this_value"
            }
        }
    }
    write_schema("enum_heterogeneous", schema)

def main():
    print("Generating stress test schemas...")
    gen_deeply_nested(50) # Very deep
    gen_heavy_polymorphism()
    gen_recursive_structures()
    gen_string_constraints()
    gen_numeric_constraints()
    gen_array_madness()
    gen_object_addprops()
    gen_enum_variants()
    
    # --- Combinatorial Generation for Volume ---
    
    print("Generating combinatorial variations...")
    
    # 1. Depth & Width variations
    for depth in [5, 10, 50]:
        for width in [2, 5]:
            schema = {"type": "object", "properties": {}, "required": []}
            current = schema
            for d in range(depth):
                prop = f"level_{d}"
                current["properties"][prop] = {"type": "object", "properties": {}, "required": []}
                current["required"].append(prop)
                # Expand width at this level
                for w in range(width):
                    current["properties"][prop]["properties"][f"item_{w}"] = {"type": "string"}
                current = current["properties"][prop]
            write_schema(f"combo_depth_{depth}_width_{width}", schema)

    # 2. Array permutations
    types = ["string", "integer", "boolean", "null"]
    for i in range(10):
        # Random tuple composition
        prefix = [{"type": random.choice(types)} for _ in range(random.randint(1, 5))]
        schema = {
            "type": "array",
            "prefixItems": prefix,
            "items": {"type": "string"}, # open ended
            "minItems": len(prefix),
            "maxItems": len(prefix) + 5
        }
        write_schema(f"combo_array_tuple_{i}", schema)

    # 3. Polymorphism mixtures (oneOf nested in allOf)
    for i in range(10):
        schema = {
            "allOf": [
                {
                    "type": "object",
                    "properties": {"common": {"const": f"val_{i}"}},
                    "required": ["common"]
                },
                {
                    "oneOf": [
                        {"type": "object", "properties": {"a": {"type": "string"}}},
                        {"type": "object", "properties": {"b": {"type": "integer"}}}
                    ]
                }
            ]
        }
        write_schema(f"combo_poly_mix_{i}", schema)

    # 4. Edge Cases: "The Void"
    write_schema("edge_true", True)
    write_schema("edge_false", False)
    write_schema("edge_empty", {})
    write_schema("edge_not_string", {"not": {"type": "string"}})
    
    # 5. Dangerous Property Names (reserved keywords in some langs)
    reserved = ["class", "int", "float", "return", "function", "var", "let", "const", "try", "catch"]
    schema = {
        "type": "object",
        "properties": {k: {"type": "string"} for k in reserved},
        "required": reserved
    }
    write_schema("edge_reserved_words", schema)

    # 6. Unicode Madness
    unicode_keys = ["🚀", "你好", "☃️", "a list"] # with space
    schema = {
        "type": "object",
        "properties": {k: {"type": "string"} for k in unicode_keys},
        "required": unicode_keys
    }
    write_schema("edge_unicode_keys", schema)

    # Fill remainder with random recursive structures
    existing = len(os.listdir(OUTPUT_DIR))
    target = 100
    for i in range(target - existing):
        # Random tree
        schema = {
            "$defs": {
                f"node_{i}": {
                    "type": "object",
                    "properties": {
                        "val": {"type": "integer"},
                        "next": {"$ref": f"#/$defs/node_{i}"},
                        "sibling": {"oneOf": [{"type": "null"}, {"$ref": f"#/$defs/node_{i}"}]}
                    }
                }
            },
            "$ref": f"#/$defs/node_{i}"
        }
        write_schema(f"combo_recursive_random_{i}", schema)

    print(f"Done. Total schemas in {OUTPUT_DIR}: {len(os.listdir(OUTPUT_DIR))}")

if __name__ == "__main__":
    main()
