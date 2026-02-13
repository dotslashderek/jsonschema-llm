//! Property-based test for the convert→rehydrate roundtrip invariant.
//!
//! Generates JSON Schemas with primitives, one level of nesting, and arrays
//! of primitives. Couples schema→data generation using `prop_map` to ensure
//! data conforms to the generated schema.
//!
//! Invariant: for identity-preserving schemas (no maps, polymorphism, or
//! recursion), `rehydrate(data, convert(schema).codec, schema).data == data`
//! for any data conforming to the schema, since the converted schema is
//! structurally identical to the original.

use jsonschema_llm_core::{convert, rehydrate, ConvertOptions};
use proptest::prelude::*;
use serde_json::{json, Map, Value};

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// Leaf type: string, integer, number, or boolean.
#[derive(Debug, Clone)]
enum LeafType {
    String,
    Integer,
    Number,
    Boolean,
}

/// Generate a leaf type.
fn arb_leaf_type() -> impl Strategy<Value = LeafType> {
    prop_oneof![
        Just(LeafType::String),
        Just(LeafType::Integer),
        Just(LeafType::Number),
        Just(LeafType::Boolean),
    ]
}

/// Property type: leaf, array of leaf, or nested object (depth-limited).
#[derive(Debug, Clone)]
enum PropType {
    Leaf(LeafType),
    ArrayOf(LeafType),
    NestedObject(Vec<(String, LeafType)>),
}

/// Generate a valid property name: [a-zA-Z_][a-zA-Z0-9_]{0,15}
fn arb_prop_name() -> impl Strategy<Value = String> {
    "[a-zA-Z_][a-zA-Z0-9_]{0,10}"
}

/// Generate conforming data for a leaf type.
fn data_for_leaf(leaf: &LeafType) -> Value {
    match leaf {
        LeafType::String => json!("test_value"),
        LeafType::Integer => json!(42),
        LeafType::Number => json!(1.23),
        LeafType::Boolean => json!(true),
    }
}

/// Generate the JSON Schema fragment for a leaf type.
fn schema_for_leaf(leaf: &LeafType) -> Value {
    match leaf {
        LeafType::String => json!({"type": "string"}),
        LeafType::Integer => json!({"type": "integer"}),
        LeafType::Number => json!({"type": "number"}),
        LeafType::Boolean => json!({"type": "boolean"}),
    }
}

/// Generate a property type.
fn arb_prop_type() -> impl Strategy<Value = PropType> {
    prop_oneof![
        6 => arb_leaf_type().prop_map(PropType::Leaf),
        2 => arb_leaf_type().prop_map(PropType::ArrayOf),
        2 => proptest::collection::vec((arb_prop_name(), arb_leaf_type()), 1..=3)
            .prop_map(PropType::NestedObject),
    ]
}

/// Build a schema `Value` and conforming data `Value` for a given property type.
fn schema_and_data_for_prop(prop_type: &PropType) -> (Value, Value) {
    match prop_type {
        PropType::Leaf(leaf) => (schema_for_leaf(leaf), data_for_leaf(leaf)),
        PropType::ArrayOf(leaf) => {
            let schema = json!({
                "type": "array",
                "items": schema_for_leaf(leaf),
            });
            let data = json!([data_for_leaf(leaf)]);
            (schema, data)
        }
        PropType::NestedObject(fields) => {
            let mut props = Map::new();
            let mut data_obj = Map::new();
            let mut required = Vec::new();
            for (name, leaf) in fields {
                if !props.contains_key(name) {
                    props.insert(name.clone(), schema_for_leaf(leaf));
                    data_obj.insert(name.clone(), data_for_leaf(leaf));
                    required.push(json!(name));
                }
            }
            let schema = json!({
                "type": "object",
                "properties": Value::Object(props),
                "required": required,
                "additionalProperties": false,
            });
            (schema, Value::Object(data_obj))
        }
    }
}

/// Generate a complete (schema, conforming_data) pair.
fn arb_schema_and_data() -> impl Strategy<Value = (Value, Value)> {
    proptest::collection::vec((arb_prop_name(), arb_prop_type()), 1..=6).prop_map(|fields| {
        let mut properties = Map::new();
        let mut data_obj = Map::new();
        let mut required = Vec::new();

        for (name, prop_type) in &fields {
            if !properties.contains_key(name) {
                let (schema_fragment, data_fragment) = schema_and_data_for_prop(prop_type);
                properties.insert(name.clone(), schema_fragment);
                data_obj.insert(name.clone(), data_fragment);
                required.push(json!(name));
            }
        }

        let schema = json!({
            "type": "object",
            "properties": Value::Object(properties),
            "required": required,
            "additionalProperties": false,
        });

        (schema, Value::Object(data_obj))
    })
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..Default::default() })]

    /// Property: convert→rehydrate roundtrip preserves identity for simple schemas.
    ///
    /// For schemas that don't trigger structural transforms (maps, polymorphism,
    /// recursion), the rehydrated data must equal the original conforming data.
    #[test]
    fn convert_rehydrate_roundtrip((schema, data) in arb_schema_and_data()) {
        let options = ConvertOptions::default();

        // Convert must succeed
        let result = convert(&schema, &options)
            .expect("convert should not fail on well-formed schema");

        // Rehydrate must succeed
        let rehydrated = rehydrate(&data, &result.codec, &schema)
            .expect("rehydrate should not fail with matching codec");

        // Identity: rehydrated data == original data
        prop_assert_eq!(
            &rehydrated.data, &data,
            "roundtrip should preserve data identity"
        );
    }

    /// Property: convert never panics on arbitrary simple schemas.
    #[test]
    fn convert_no_panic((schema, _data) in arb_schema_and_data()) {
        let options = ConvertOptions::default();
        let _ = convert(&schema, &options);
    }
}
