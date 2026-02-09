//! Integration tests for the `convert()` pipeline — exercises the full 8-pass chain
//! via the public API only, never calling individual passes directly.

use jsonschema_llm_core::codec::Transform;
use jsonschema_llm_core::{convert, rehydrate, ConvertOptions, Target};
use serde_json::json;

fn openai_options() -> ConvertOptions {
    ConvertOptions::default() // OpenaiStrict, AnyOf, depth 50, recursion 3
}

fn gemini_options() -> ConvertOptions {
    ConvertOptions {
        target: Target::Gemini,
        ..ConvertOptions::default()
    }
}

// ── Basic Pipeline ──────────────────────────────────────────────────────────

#[test]
fn test_convert_simple_schema() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "integer" }
        },
        "required": ["name"]
    });

    let result = convert(&schema, &openai_options()).expect("convert should succeed");

    // Strict mode: additionalProperties: false, all props required,
    // optional `age` wrapped as anyOf [type, null]
    let props = result.schema["properties"]
        .as_object()
        .expect("should have properties");
    assert!(props.contains_key("name"));
    assert!(props.contains_key("age"));
    assert_eq!(result.schema["additionalProperties"], json!(false));

    let required = result.schema["required"]
        .as_array()
        .expect("should have required");
    assert!(required.contains(&json!("name")));
    assert!(required.contains(&json!("age")));
}

// ── Dictionary / Map Transpilation ──────────────────────────────────────────

#[test]
fn test_convert_with_map() {
    let schema = json!({
        "type": "object",
        "properties": {
            "tags": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            }
        },
        "required": ["tags"]
    });

    let result = convert(&schema, &openai_options()).expect("convert should succeed");

    // `tags` should be transpiled from map → array of {key, value}
    let tags = &result.schema["properties"]["tags"];
    assert_eq!(tags["type"], json!("array"));
    assert!(tags["items"]["properties"].as_object().is_some());

    // Codec should contain a MapToArray transform
    assert!(
        result
            .codec
            .transforms
            .iter()
            .any(|t| matches!(t, Transform::MapToArray { .. })),
        "codec should contain MapToArray transform"
    );
}

// ── Opaque Object → String ──────────────────────────────────────────────────

#[test]
fn test_convert_with_opaque() {
    let schema = json!({
        "type": "object",
        "properties": {
            "metadata": { "type": "object" }
        },
        "required": ["metadata"]
    });

    let result = convert(&schema, &openai_options()).expect("convert should succeed");

    // Opaque `metadata` should become string
    let _metadata = &result.schema["properties"]["metadata"];
    // After strict wrapping, the type should ultimately be string (or anyOf wrapping string)
    // The key thing is that a JsonStringParse transform exists
    assert!(
        result
            .codec
            .transforms
            .iter()
            .any(|t| matches!(t, Transform::JsonStringParse { .. })),
        "codec should contain JsonStringParse transform for opaque object"
    );
}

// ── allOf Composition ───────────────────────────────────────────────────────

#[test]
fn test_convert_with_allof() {
    let schema = json!({
        "allOf": [
            {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" }
                },
                "required": ["id"]
            },
            {
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                },
                "required": ["name"]
            }
        ]
    });

    let result = convert(&schema, &openai_options()).expect("convert should succeed");

    // allOf should be merged into a flat object with both id and name
    let props = result.schema["properties"]
        .as_object()
        .expect("should have properties");
    assert!(props.contains_key("id"), "merged should have 'id'");
    assert!(props.contains_key("name"), "merged should have 'name'");
    assert!(
        result.schema.get("allOf").is_none(),
        "allOf should be removed after merge"
    );
}

// ── oneOf → anyOf Polymorphism ──────────────────────────────────────────────

#[test]
fn test_convert_with_oneof() {
    let schema = json!({
        "oneOf": [
            { "type": "string" },
            { "type": "integer" }
        ]
    });

    let result = convert(&schema, &openai_options()).expect("convert should succeed");

    // oneOf should be rewritten to anyOf for OpenAI
    assert!(
        result.schema.get("oneOf").is_none(),
        "oneOf should be removed"
    );
    assert!(
        result.schema.get("anyOf").is_some(),
        "anyOf should be present"
    );
}

// ── $ref Resolution ─────────────────────────────────────────────────────────

#[test]
fn test_convert_with_refs() {
    let schema = json!({
        "type": "object",
        "properties": {
            "address": { "$ref": "#/$defs/Address" }
        },
        "required": ["address"],
        "$defs": {
            "Address": {
                "type": "object",
                "properties": {
                    "street": { "type": "string" },
                    "city": { "type": "string" }
                },
                "required": ["street", "city"]
            }
        }
    });

    let result = convert(&schema, &openai_options()).expect("convert should succeed");

    // $ref should be inlined — the address property should have actual properties
    let address = &result.schema["properties"]["address"];
    let addr_props = address["properties"]
        .as_object()
        .expect("address should have inlined properties");
    assert!(addr_props.contains_key("street"));
    assert!(addr_props.contains_key("city"));
}

// ── Recursive Schema Breaking ───────────────────────────────────────────────

#[test]
fn test_convert_with_recursion() {
    let schema = json!({
        "$ref": "#/$defs/TreeNode",
        "$defs": {
            "TreeNode": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "children": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/TreeNode" }
                    }
                },
                "required": ["name"]
            }
        }
    });

    let options = ConvertOptions {
        recursion_limit: 2,
        ..openai_options()
    };
    let result = convert(&schema, &options).expect("convert should succeed");

    // The schema should not infinitely recurse — at some depth, the recursive
    // ref should be broken with a string placeholder
    // Just verify it doesn't panic and produces a valid result
    assert!(result.schema["properties"].as_object().is_some());

    // There should be RecursiveInflate transforms in the codec
    let serialized = serde_json::to_string(&result.schema).unwrap();
    assert!(
        !serialized.contains("\"$ref\""),
        "all $ref nodes should be resolved or broken"
    );
}

// ── Full Roundtrip ──────────────────────────────────────────────────────────

#[test]
fn test_convert_full_roundtrip() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "tags": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            }
        },
        "required": ["name", "tags"]
    });

    let result = convert(&schema, &openai_options()).expect("convert should succeed");

    // Simulate LLM output conforming to the converted schema
    // tags was transpiled to array — LLM returns array of {key, value}
    let llm_output = json!({
        "name": "test",
        "tags": [
            { "key": "env", "value": "prod" },
            { "key": "team", "value": "platform" }
        ]
    });

    let rehydrated = rehydrate(&llm_output, &result.codec).expect("rehydrate should succeed");

    // After rehydration, tags should be back to a map
    let tags = &rehydrated.data["tags"];
    assert_eq!(tags["env"], json!("prod"));
    assert_eq!(tags["team"], json!("platform"));
}

// ── Target-Specific Skips ───────────────────────────────────────────────────

#[test]
fn test_gemini_skips_passes() {
    let schema = json!({
        "type": "object",
        "properties": {
            "tags": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            }
        },
        "required": ["tags"]
    });

    let result = convert(&schema, &gemini_options()).expect("convert should succeed");

    // Gemini skips Pass 3 (dictionary) — tags should NOT be transpiled to array
    let tags = &result.schema["properties"]["tags"];
    assert_ne!(
        tags["type"],
        json!("array"),
        "Gemini should not transpile maps"
    );

    // Gemini skips dictionary pass so no MapToArray transforms
    assert!(
        !result
            .codec
            .transforms
            .iter()
            .any(|t| matches!(t, Transform::MapToArray { .. })),
        "Gemini should have no MapToArray transforms"
    );
}
