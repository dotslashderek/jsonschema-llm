//! Pass 9 — Provider compatibility transforms for OpenAI Strict Mode.
//!
//! Runs **after** all other passes (the schema is already normalized, refs resolved,
//! strict-sealed, etc.) and both transforms and emits advisory `ProviderCompatError`s
//! for constructs that would be rejected by the target provider.
//!
//! Active only when `target == OpenaiStrict && mode == Strict`.
//!
//! ## Checks
//!
//! | Issue | Check                  | Kind       |
//! | ----- | ---------------------- | ---------- |
//! | #94   | Root type enforcement  | Transform  |
//! | #95   | Depth budget           | Diagnostic |
//! | #96   | Enum homogeneity       | Transform  |
//! | #97   | Boolean / empty schema | Transform  |

use crate::codec::Transform;
use crate::config::{ConvertOptions, Mode, Target};
use crate::error::ProviderCompatError;
use crate::schema_utils::{build_opaque_description, build_path};
use serde_json::{json, Value};

use super::pass_result::PassResult;
use super::pass_utils::{enforce_object_strict, extract_types};

/// OpenAI Strict Mode maximum nesting depth.
///
/// OpenAI enforces a 10-level nesting limit for structured output schemas.
/// Schemas exceeding this are rejected with:
///   "N levels of nesting exceeds limit of 10"
///
/// Our `semantic_depth` counter tracks data-shape edges (properties, items,
/// additionalProperties, etc.) which maps 1:1 to OpenAI's nesting count.
/// Combinators (anyOf, oneOf, allOf) do NOT increment semantic depth.
const OPENAI_MAX_DEPTH: usize = 10;

/// Hard guard against infinite recursion in traversal.
const HARD_RECURSION_LIMIT: usize = 100;

/// Result of provider compatibility checks.
pub struct ProviderCompatResult {
    /// Shared pass result containing the (possibly modified) schema and transforms.
    pub pass: PassResult,
    /// Advisory errors for provider-incompatible constructs.
    pub errors: Vec<ProviderCompatError>,
}

/// Run all provider compatibility checks on the post-pipeline schema.
///
/// Returns the (potentially wrapped) schema, any new transforms, and
/// advisory errors.
pub fn check_provider_compat(schema: Value, config: &ConvertOptions) -> ProviderCompatResult {
    match config.target {
        Target::OpenaiStrict if config.mode == Mode::Strict => {
            let mut errors = Vec::new();
            let mut transforms = Vec::new();

            // ── Check 1: Root type enforcement (#94) ──────────────────
            let mut schema = check_root_type(schema, config.target, &mut errors, &mut transforms);

            // ── Checks 2–4: Single-pass mutating visitor (#95, #96, #97)
            {
                let mut visitor = CompatVisitor {
                    errors: &mut errors,
                    transforms: &mut transforms,
                    target: config.target,
                    max_depth_observed: 0,
                };
                visitor.visit(&mut schema, "#", 0, 0);
            }

            // (#95 truncation emits per-path DepthBudgetExceeded errors inline)

            ProviderCompatResult {
                pass: PassResult::with_transforms(schema, transforms),
                errors,
            }
        }
        _ => ProviderCompatResult {
            pass: PassResult::schema_only(schema),
            errors: vec![],
        },
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Check 1: Root type enforcement (#94)
// ═══════════════════════════════════════════════════════════════════════════

/// Wraps non-object roots or roots with combinators in
/// `{ type: object, properties: { result: <original> }, ... }`.
///
/// OpenAI strict mode requires the root schema to:
///   1. Have `type: "object"`
///   2. NOT have `anyOf`/`oneOf`/`allOf`/`not`/`enum` at the top level
fn check_root_type(
    schema: Value,
    target: Target,
    errors: &mut Vec<ProviderCompatError>,
    transforms: &mut Vec<Transform>,
) -> Value {
    let root_types = extract_types(&schema);
    let is_object = root_types.len() == 1 && root_types[0] == "object";

    // Check for root-level combinators that OpenAI rejects
    let combinator_keywords = ["anyOf", "oneOf", "allOf", "not", "enum"];
    let has_root_combinator = combinator_keywords
        .iter()
        .any(|kw| schema.get(*kw).is_some());

    // Strict: only skip wrapping if type is exactly "object" AND no root combinators
    if is_object && !has_root_combinator {
        return schema;
    }

    // Determine the reason for wrapping
    if !is_object {
        let actual_type = if root_types.is_empty() {
            "unspecified".to_string()
        } else {
            root_types.join(", ")
        };
        errors.push(ProviderCompatError::RootTypeIncompatible {
            actual_type: actual_type.clone(),
            target,
            hint: format!(
                "Schema root type '{}' is not 'object'. Wrapping in {{ \"result\": <original> }}.",
                actual_type,
            ),
        });
    }

    if has_root_combinator {
        let found: Vec<&str> = combinator_keywords
            .iter()
            .filter(|kw| schema.get(**kw).is_some())
            .copied()
            .collect();
        errors.push(ProviderCompatError::RootTypeIncompatible {
            actual_type: found.join(", "),
            target,
            hint: format!(
                "Root schema has {} which OpenAI strict mode forbids at the top level. Wrapping.",
                found.join("/"),
            ),
        });
    }

    transforms.push(Transform::RootObjectWrapper {
        path: "#".to_string(),
        wrapper_key: "result".to_string(),
    });

    // Build the wrapper schema
    let mut wrapper = json!({
        "type": "object",
        "properties": {
            "result": schema,
        },
        "required": ["result"],
        "additionalProperties": false,
    });

    // #110: If the inner schema has `properties`, apply strict enforcement.
    // This handles schemas that had properties but no `type: object` — p6
    // skipped them because it gates on is_typed_object.
    //
    // We run unconditionally when properties exist because enforce_object_strict
    // is idempotent: already-nullable props are not double-wrapped,
    // already-required keys stay required, and additionalProperties: false is
    // a no-op if already set.
    //
    // Transform path must be "#/properties/result" (the physical wrapped path)
    // because the rehydrator applies transforms LIFO: NullableOptional runs
    // BEFORE RootObjectWrapper unwraps.
    if let Some(inner) = wrapper
        .pointer_mut("/properties/result")
        .and_then(|v| v.as_object_mut())
    {
        let has_properties = inner
            .get("properties")
            .and_then(|v| v.as_object())
            .is_some_and(|p| !p.is_empty());

        if has_properties {
            enforce_object_strict(inner, "#/properties/result", transforms);
        }
    }

    wrapper
}

// ═══════════════════════════════════════════════════════════════════════════
// Checks 2–4: Single-pass CompatVisitor
//
// NOTE: When adding keywords here, also update `schema_utils::recurse_into_children`.
// Conversely, when adding keywords to `recurse_into_children`, update this visitor.
// ═══════════════════════════════════════════════════════════════════════════

struct CompatVisitor<'a> {
    errors: &'a mut Vec<ProviderCompatError>,
    transforms: &'a mut Vec<Transform>,
    target: Target,
    max_depth_observed: usize,
}

impl CompatVisitor<'_> {
    /// Recursively visit and **mutate** a schema node.
    ///
    /// Uses two depth counters:
    /// - `recursion_depth`: **always** increments — guards against stack overflow
    ///   via `HARD_RECURSION_LIMIT`.
    /// - `semantic_depth`: increments only on data-shape edges (`properties`, `items`,
    ///   `additionalProperties`, etc.) — used for `OPENAI_MAX_DEPTH` budget.
    ///
    /// - #95 Depth budget: diagnostic only (tracks max semantic depth)
    /// - #96 Enum homogeneity: **transforms** mixed enums → all strings + codec entry
    /// - #97 Boolean/empty schemas: **transforms** → opaque string or sealed empty object
    fn visit(
        &mut self,
        schema: &mut Value,
        path: &str,
        recursion_depth: usize,
        semantic_depth: usize,
    ) {
        // Hard recursion guard (always uses recursion_depth)
        if recursion_depth > HARD_RECURSION_LIMIT {
            return;
        }

        // ── #97: Boolean schema transform ──────────────────────────
        if let Some(b) = schema.as_bool() {
            self.errors.push(ProviderCompatError::UnconstrainedSchema {
                path: path.to_string(),
                schema_kind: format!("boolean({})", b),
                target: self.target,
                hint: format!("Boolean schema '{}' replaced with opaque string.", b),
            });

            *schema = json!({
                "type": "string",
                "description": "MUST be a valid JSON object serialized as a string, e.g. \"{\\\"key\\\": \\\"value\\\"}\". Do NOT output plain text — the value must parse with JSON.parse()."
            });
            self.transforms.push(Transform::JsonStringParse {
                path: path.to_string(),
            });
            return;
        }

        match schema.as_object() {
            Some(_) => {}
            None => return,
        };

        // ── #95 Depth budget (diagnostic, uses semantic_depth) ─────
        if semantic_depth > self.max_depth_observed {
            self.max_depth_observed = semantic_depth;
        }

        // ── #95 Depth budget: TRUNCATE at limit ───────────────
        // Only truncate schemas that contribute to nesting (objects, arrays).
        // Primitive leaves (string, integer, number, boolean) don't add depth
        // and should pass through untouched.
        // Also recognises nullable-wrapped primitives from p6:
        //   anyOf: [{type: "string"}, {type: "null"}]
        if semantic_depth >= OPENAI_MAX_DEPTH && path != "#" {
            let schema_type = schema.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let is_primitive = matches!(
                schema_type,
                "string" | "integer" | "number" | "boolean" | "null"
            );
            // Check for nullable-wrapped primitive from p6 strict pass:
            // anyOf: [{type: <primitive>}, {type: "null"}]
            let is_nullable_primitive = !is_primitive
                && schema
                    .get("anyOf")
                    .and_then(|v| v.as_array())
                    .map(|variants| {
                        variants.len() == 2
                            && variants
                                .iter()
                                .any(|v| v.get("type").and_then(|t| t.as_str()) == Some("null"))
                            && variants.iter().any(|v| {
                                matches!(
                                    v.get("type").and_then(|t| t.as_str()),
                                    Some("string" | "integer" | "number" | "boolean")
                                )
                            })
                    })
                    .unwrap_or(false);
            // Also skip if the schema has no sub-structure (no properties, items, etc.)
            let has_sub_structure = schema.get("properties").is_some()
                || schema.get("items").is_some()
                || schema.get("additionalProperties").is_some()
                || schema.get("anyOf").is_some()
                || schema.get("oneOf").is_some()
                || schema.get("allOf").is_some()
                || schema.get("prefixItems").is_some();

            if (is_primitive && !has_sub_structure) || is_nullable_primitive {
                // Primitive leaf — no nesting contribution, leave it alone
                return;
            }

            // Build a structural description so the LLM knows what JSON to produce
            let desc = build_opaque_description(schema);

            self.errors.push(ProviderCompatError::DepthBudgetExceeded {
                actual_depth: semantic_depth,
                max_depth: OPENAI_MAX_DEPTH,
                target: self.target,
                hint: format!(
                    "Sub-schema at '{}' reaches depth limit {}. Replaced with opaque string.",
                    path, OPENAI_MAX_DEPTH,
                ),
            });

            *schema = json!({
                "type": "string",
                "description": desc
            });
            self.transforms.push(Transform::JsonStringParse {
                path: path.to_string(),
            });
            return;
        }

        // ── #96 Enum homogeneity (transform) ──────────────────────
        fix_enum_homogeneity(schema, path, self.target, self.errors, self.transforms);

        // ── #97 Unconstrained sub-schemas (transform) ─────────────
        if path != "#" {
            if let Some(obj) = schema.as_object() {
                if is_unconstrained(obj) {
                    self.errors.push(ProviderCompatError::UnconstrainedSchema {
                        path: path.to_string(),
                        schema_kind: "empty".to_string(),
                        target: self.target,
                        hint: "Unconstrained schema replaced with opaque string.".into(),
                    });

                    *schema = json!({
                        "type": "string",
                        "description": "MUST be a valid JSON object serialized as a string, e.g. \"{\\\"key\\\": \\\"value\\\"}\". Do NOT output plain text — the value must parse with JSON.parse()."
                    });
                    self.transforms.push(Transform::JsonStringParse {
                        path: path.to_string(),
                    });
                    return;
                }
            }
        }

        // ── #246 patternProperties → strip or opaque-stringify ────
        // OpenAI strict mode does not support `patternProperties`. Handle it
        // based on the schema shape:
        //   1. Typed object with properties → strip patternProperties (keep structure)
        //   2. Typed sterile (no properties) or untyped nested → opaque-stringify
        //   3. Root (any shape) → strip only (check_root_type already wrapped)
        if let Some(obj) = schema.as_object() {
            if obj.contains_key("patternProperties") {
                let is_typed_obj = obj.get("type").and_then(Value::as_str) == Some("object");
                let has_properties = obj
                    .get("properties")
                    .and_then(Value::as_object)
                    .is_some_and(|p| !p.is_empty());

                // Collect dropped pattern keys for the hint message.
                let dropped_keys: Vec<String> = obj
                    .get("patternProperties")
                    .and_then(Value::as_object)
                    .map(|pp| pp.keys().cloned().collect())
                    .unwrap_or_default();
                let hint_keys = dropped_keys.join(", ");

                if is_typed_obj && has_properties {
                    // Branch 1: typed with properties → strip patternProperties
                    let obj = schema.as_object_mut().unwrap();
                    obj.remove("patternProperties");
                    self.errors
                        .push(ProviderCompatError::PatternPropertiesStripped {
                            path: path.to_string(),
                            target: self.target,
                            hint: format!(
                                "Dropped patterns [{}] from typed object with explicit properties.",
                                hint_keys
                            ),
                        });
                } else if path != "#" {
                    // Branch 2: untyped OR typed-but-sterile (nested) → opaque-stringify
                    let desc = build_opaque_description(schema);
                    self.errors.push(ProviderCompatError::PatternPropertiesStringified {
                        path: path.to_string(),
                        target: self.target,
                        hint: format!(
                            "Patterns [{}] opaque-stringified (no explicit properties to preserve).",
                            hint_keys
                        ),
                    });
                    *schema = json!({
                        "type": "string",
                        "description": desc
                    });
                    self.transforms.push(Transform::JsonStringParse {
                        path: path.to_string(),
                    });
                    return; // Don't recurse into children (they're gone)
                } else {
                    // Branch 3: explicitly typed root object (e.g. type: "object" at root)
                    // that was NOT wrapped by check_root_type. Untyped roots get wrapped,
                    // placing the inner schema at #/properties/result (path != "#"),
                    // where Branch 2 handles it. This branch only fires for roots that
                    // already had type: "object" and thus bypassed wrapping.
                    let obj = schema.as_object_mut().unwrap();
                    obj.remove("patternProperties");
                    self.errors
                        .push(ProviderCompatError::PatternPropertiesStripped {
                            path: path.to_string(),
                            target: self.target,
                            hint: format!("Dropped patterns [{}] from root schema.", hint_keys),
                        });
                }
            }
        }

        // ── #246 Strip reference-mechanism keywords ───────────────
        // $anchor, $dynamicAnchor, $dynamicRef are resolution-mechanism
        // keywords with no semantic value after pipeline flattening.
        // OpenAI strict mode does not support them.
        if let Some(obj) = schema.as_object_mut() {
            for keyword in &["$anchor", "$dynamicAnchor", "$dynamicRef"] {
                if obj.remove(*keyword).is_some() {
                    self.errors.push(ProviderCompatError::RefKeywordStripped {
                        path: path.to_string(),
                        keyword: keyword.to_string(),
                        target: self.target,
                        hint: format!("{} stripped (not supported in strict mode).", keyword),
                    });
                }
            }
        }

        // ── #254 Type array → anyOf conversion ───────────────────
        // OpenAI strict mode does not support type arrays like
        // `type: ["string", "null"]`. Convert to equivalent anyOf branches.
        // Each type becomes a branch; non-type keywords stay on the parent.
        {
            let needs_conversion = schema.get("type").and_then(|v| v.as_array()).is_some();

            if needs_conversion {
                let type_arr: Vec<String> = schema
                    .get("type")
                    .and_then(|v| v.as_array())
                    .unwrap()
                    .iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect();

                if type_arr.len() > 1 {
                    let obj = schema.as_object_mut().unwrap();
                    obj.remove("type");

                    // Build anyOf branches — each type gets its own branch
                    // with relevant constraints from the parent.
                    // Bare object (no properties) and bare array (no items)
                    // would violate strict mode, so we opaque-stringify them.
                    let has_properties = obj
                        .get("properties")
                        .and_then(Value::as_object)
                        .is_some_and(|p| !p.is_empty());
                    let has_items = obj.get("items").is_some() || obj.get("prefixItems").is_some();

                    let branches: Vec<Value> = type_arr
                        .iter()
                        .map(|t| {
                            let mut branch = json!({"type": t});
                            // For object branches, carry over properties-related keywords
                            if t == "object" {
                                if has_properties {
                                    for kw in &["properties", "required", "additionalProperties"]
                                    {
                                        if let Some(val) = obj.get(*kw) {
                                            branch[*kw] = val.clone();
                                        }
                                    }
                                } else {
                                    // Bare object → opaque string
                                    return json!({
                                        "type": "string",
                                        "description": "MUST be a valid JSON object serialized as a string, e.g. \"{\\\"key\\\": \\\"value\\\"}\". Do NOT output plain text — the value must parse with JSON.parse()."
                                    });
                                }
                            }
                            // For array branches, carry over items-related keywords
                            if t == "array" {
                                if has_items {
                                    for kw in &["items", "prefixItems", "minItems", "maxItems"] {
                                        if let Some(val) = obj.get(*kw) {
                                            branch[*kw] = val.clone();
                                        }
                                    }
                                } else {
                                    // Bare array → opaque string
                                    return json!({
                                        "type": "string",
                                        "description": "MUST be a valid JSON array serialized as a string, e.g. \"[1, 2, 3]\". Do NOT output plain text — the value must parse with JSON.parse()."
                                    });
                                }
                            }
                            branch
                        })
                        .collect();

                    // Remove keywords that were moved into branches
                    for kw in &[
                        "properties",
                        "required",
                        "additionalProperties",
                        "items",
                        "prefixItems",
                        "minItems",
                        "maxItems",
                    ] {
                        obj.remove(*kw);
                    }

                    obj.insert("anyOf".to_string(), Value::Array(branches));

                    self.errors.push(ProviderCompatError::TypeArrayConverted {
                        path: path.to_string(),
                        types: type_arr,
                        target: self.target,
                        hint: "Type array converted to anyOf branches.".to_string(),
                    });
                }
            }
        }

        // ── Recurse into children ──────────────────────────────────
        // Data-shape keywords increment semantic_depth.
        // Non-data-shape keywords (combinators, conditionals, defs) do not.
        // All keywords always increment recursion_depth.

        let rd = recursion_depth + 1; // next recursion depth (always +1)
        let sd_data = semantic_depth + 1; // next semantic depth for data-shape edges
        let sd_same = semantic_depth; // unchanged semantic depth for non-data edges

        // ── Data-shape: map-of-schemas ─────────────────────────────
        // properties, patternProperties
        for keyword in &["properties", "patternProperties"] {
            let keys: Vec<String> = schema
                .get(*keyword)
                .and_then(|v| v.as_object())
                .map(|props| props.keys().cloned().collect())
                .unwrap_or_default();
            for key in &keys {
                let child_path = build_path(path, &[keyword, key]);
                if let Some(child) = schema.get_mut(*keyword).and_then(|p| p.get_mut(key)) {
                    self.visit(child, &child_path, rd, sd_data);
                }
            }
        }

        // ── Data-shape: single-schema ──────────────────────────────
        // additionalProperties, unevaluatedProperties, unevaluatedItems, contains
        // Visit objects and `true` booleans (unconstrained). Skip `false` (sealed constraint).
        for keyword in &[
            "additionalProperties",
            "unevaluatedProperties",
            "unevaluatedItems",
            "contains",
        ] {
            let should_visit = schema
                .get(*keyword)
                .map(|v| v.is_object() || v.as_bool() == Some(true))
                .unwrap_or(false);
            if should_visit {
                let child_path = build_path(path, &[keyword]);
                if let Some(child) = schema.get_mut(*keyword) {
                    self.visit(child, &child_path, rd, sd_data);
                }
            }
        }

        // ── Data-shape: items (single-schema, tuple array, or boolean) ──
        // Handle `items: {schema}`, `items: [{schema}, ...]`, and `items: true` (unconstrained).
        // Skip `items: false` (deny all items = intentional constraint).
        // (cf. schema_utils::recurse_into_children for the canonical list)
        {
            // Determine shape without holding a mutable borrow
            let is_obj = schema.get("items").is_some_and(|v| v.is_object());
            let is_true = schema.get("items").and_then(|v| v.as_bool()) == Some(true);
            let tuple_len = schema
                .get("items")
                .and_then(|v| v.as_array())
                .map(|a| a.len());

            if is_obj || is_true {
                // Single schema or boolean `true` (unconstrained)
                let child_path = build_path(path, &["items"]);
                if let Some(child) = schema.get_mut("items") {
                    self.visit(child, &child_path, rd, sd_data);
                }
            } else if let Some(count) = tuple_len {
                // Tuple array — iterate each positional item
                for i in 0..count {
                    let child_path = build_path(path, &["items", &i.to_string()]);
                    if let Some(child) = schema.get_mut("items").and_then(|p| p.get_mut(i)) {
                        self.visit(child, &child_path, rd, sd_data);
                    }
                }
            }
        }

        // ── Data-shape: array-of-schemas ───────────────────────────
        // prefixItems
        {
            let count = schema
                .get("prefixItems")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            for i in 0..count {
                let child_path = build_path(path, &["prefixItems", &i.to_string()]);
                if let Some(child) = schema.get_mut("prefixItems").and_then(|p| p.get_mut(i)) {
                    self.visit(child, &child_path, rd, sd_data);
                }
            }
        }

        // ── #122 / #140: Replace conflicting `items` with permissive union ──
        // When `prefixItems` has mixed types and `items` declares a single
        // conflicting type, OpenAI strict mode still *requires* `items` to be
        // present on array schemas. Removing it (the original approach in
        // b3f9414) broke structured output for all combo_array_tuple_* schemas.
        //
        // Fix: replace `items` with `anyOf` of the distinct types from
        // `prefixItems`, so the schema satisfies OpenAI's structural
        // requirement without contradicting positional types.
        //
        // Conservative: only compare entries that have an explicit `type` field.
        // Entries using `$ref`, combinators, or other constructs are ignored
        // to avoid false positives.
        {
            let replacement = (|| {
                let prefix_arr = schema.get("prefixItems")?.as_array()?;
                if prefix_arr.is_empty() {
                    return None;
                }
                let items_type = schema.get("items")?.get("type")?.as_str()?;
                let typed_entries: Vec<&str> = prefix_arr
                    .iter()
                    .filter_map(|pi| pi.get("type").and_then(|t| t.as_str()))
                    .collect();
                if typed_entries.is_empty() {
                    return None;
                }
                let has_conflict = !typed_entries.iter().all(|t| *t == items_type);
                if !has_conflict {
                    return None;
                }
                // Build a deduplicated anyOf from the distinct types
                let mut seen = std::collections::BTreeSet::new();
                for t in &typed_entries {
                    seen.insert(*t);
                }
                // Also include the original items type
                seen.insert(items_type);
                let variants: Vec<Value> = seen.iter().map(|t| json!({"type": *t})).collect();
                Some(json!({"anyOf": variants}))
            })();
            if let Some(union_items) = replacement {
                if let Some(obj) = schema.as_object_mut() {
                    obj.insert("items".to_string(), union_items);
                }
            }
        }

        // ── #246 Strip bare-required anyOf/oneOf branches ──────────
        // Bare-required branches like `{"required": ["paths"]}` in anyOf are
        // validation-only constraints. In strict mode, p6 makes all properties
        // required-but-nullable. To preserve the "at least one of" semantic for
        // the LLM without violating strict mode, we strip these branches and
        // append the rule to the schema's description.
        for keyword in &["anyOf", "oneOf"] {
            if let Some(arr) = schema.get(*keyword).and_then(|v| v.as_array()) {
                let bare_indices: Vec<usize> = arr
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| is_bare_required_only(v))
                    .map(|(i, _)| i)
                    .collect();

                if !bare_indices.is_empty() {
                    let mut stripped_fields: Vec<String> = bare_indices
                        .iter()
                        .filter_map(|i| {
                            arr[*i]
                                .get("required")
                                .and_then(|r| r.as_array())
                                .map(|reqs| {
                                    reqs.iter()
                                        .filter_map(|v| v.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                })
                        })
                        .collect();

                    stripped_fields.sort();
                    stripped_fields.dedup();

                    let all_bare = bare_indices.len() == arr.len();

                    if all_bare {
                        if let Some(obj) = schema.as_object_mut() {
                            obj.remove(*keyword);
                        }
                    } else if let Some(arr_mut) =
                        schema.get_mut(*keyword).and_then(|v| v.as_array_mut())
                    {
                        for i in bare_indices.iter().rev() {
                            arr_mut.remove(*i);
                        }
                    }

                    // Now, inject the description into the schema so the LLM knows
                    let hint_text = format!(
                        "Note: While properties may be nullable, a valid output MUST include a non-null value for at least one of these properties: {}",
                        stripped_fields.join(", ")
                    );

                    if let Some(obj) = schema.as_object_mut() {
                        let new_desc = match obj.get("description").and_then(|d| d.as_str()) {
                            Some(existing) => format!("{}\n\n{}", existing, hint_text),
                            None => hint_text.clone(),
                        };
                        obj.insert("description".to_string(), json!(new_desc));
                    }

                    self.errors.push(ProviderCompatError::BareRequiredStripped {
                        path: path.to_string(),
                        target: self.target,
                        hint: format!(
                            "Stripped bare-required {} branches referencing [{}]. \
                             Injected plain-text constraint into description.",
                            keyword,
                            stripped_fields.join(", ")
                        ),
                    });
                }
            }
        }

        // ── Non-data-shape: array-of-schemas (combinators) ────────
        // anyOf, oneOf, allOf
        for keyword in &["anyOf", "oneOf", "allOf"] {
            let count = schema
                .get(*keyword)
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            for i in 0..count {
                let child_path = build_path(path, &[keyword, &i.to_string()]);
                if let Some(child) = schema.get_mut(*keyword).and_then(|v| v.get_mut(i)) {
                    self.visit(child, &child_path, rd, sd_same);
                }
            }
        }

        // ── Non-data-shape: single-schema (conditionals, negation) ─
        // if, then, else, not, propertyNames
        for keyword in &["if", "then", "else", "not", "propertyNames"] {
            let has = schema
                .get(*keyword)
                .map(|v| v.is_object() || v.is_boolean())
                .unwrap_or(false);
            if has {
                let child_path = build_path(path, &[keyword]);
                if let Some(child) = schema.get_mut(*keyword) {
                    self.visit(child, &child_path, rd, sd_same);
                }
            }
        }

        // ── Non-data-shape: map-of-schemas ────────────────────────
        // $defs, definitions, dependentSchemas
        for keyword in &["$defs", "definitions", "dependentSchemas"] {
            let keys: Vec<String> = schema
                .get(*keyword)
                .and_then(|v| v.as_object())
                .map(|obj| obj.keys().cloned().collect())
                .unwrap_or_default();
            for key in &keys {
                let child_path = build_path(path, &[keyword, key]);
                if let Some(child) = schema.get_mut(*keyword).and_then(|v| v.get_mut(key)) {
                    self.visit(child, &child_path, rd, sd_same);
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Transform helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Detect mixed-type enums and stringify all values in-place.
///
/// Produces an `EnumStringify` codec entry so the rehydrator can reverse it.
fn fix_enum_homogeneity(
    schema: &mut Value,
    path: &str,
    target: Target,
    errors: &mut Vec<ProviderCompatError>,
    transforms: &mut Vec<Transform>,
) {
    let enum_vals = match schema.get("enum").and_then(|v| v.as_array()) {
        Some(vals) if !vals.is_empty() => vals,
        _ => return,
    };

    let mut types = std::collections::BTreeSet::new();
    for v in enum_vals {
        types.insert(json_type_name(v));
    }

    if types.len() <= 1 {
        return; // Homogeneous — nothing to fix
    }

    let types_found: Vec<String> = types.into_iter().map(|s| s.to_string()).collect();
    let original_values: Vec<Value> = enum_vals.clone();

    // Stringify all values and deduplicate (e.g. [1, "1"] → ["1"] not ["1", "1"])
    let mut seen = std::collections::HashSet::new();
    let stringified: Vec<Value> = original_values
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .filter(|s| seen.insert(s.clone()))
        .map(Value::String)
        .collect();

    // Replace enum in-place
    if let Some(obj) = schema.as_object_mut() {
        obj.insert("enum".to_string(), Value::Array(stringified));
        // Ensure type is string since all values are now strings
        obj.insert("type".to_string(), json!("string"));
    }

    errors.push(ProviderCompatError::MixedEnumTypes {
        path: path.to_string(),
        types_found,
        target,
        hint: "Mixed-type enum values normalized to strings.".into(),
    });

    transforms.push(Transform::EnumStringify {
        path: path.to_string(),
        original_values,
    });
}

/// Returns the JSON type name for a value.
fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Returns true if a schema value is a bare-required-only constraint:
/// `{"required": ["field1", ...]}` with no other structural keywords.
///
/// These appear as `anyOf` discriminator branches in specs like OAS31
/// (e.g. "must have at least one of: paths, components, webhooks").
/// They can't be expressed in strict mode and are redundant after p6
/// makes all properties required.
fn is_bare_required_only(v: &Value) -> bool {
    let obj = match v.as_object() {
        Some(o) => o,
        None => return false,
    };

    // Must have `required`
    if !obj.contains_key("required") {
        return false;
    }

    for key in obj.keys() {
        let k = key.as_str();
        // Allow ONLY `required` and strictly non-constraining metadata.
        // Any other keyword (including unknown ones) disqualifies it from being "bare".
        if ![
            "required",
            "description",
            "title",
            "$comment",
            "$id",
            "$anchor",
        ]
        .contains(&k)
        {
            return false;
        }
    }

    true
}

/// Returns true if a schema object is unconstrained:
///   - it is completely empty, or
///   - it only contains metadata / documentation keywords and the structural
///     keywords injected by `p6_strict` (e.g. `additionalProperties`, `required`).
///
/// Any other keyword is treated as potentially constraining, so the schema will
/// not be considered unconstrained even if we do not explicitly recognise it.
fn is_unconstrained(obj: &serde_json::Map<String, Value>) -> bool {
    // Quick check: truly empty
    if obj.is_empty() {
        return true;
    }

    // Keywords that are purely metadata / documentation and do not constrain
    // the set of acceptable instances.
    const METADATA_KEYWORDS: &[&str] = &[
        "title",
        "description",
        "$schema",
        "$id",
        "$anchor",
        "$comment",
        "examples",
        "default",
        "deprecated",
        "readOnly",
        "writeOnly",
        "contentMediaType",
        "contentEncoding",
        "contentSchema",
    ];

    // If we see any key that is not known-metadata, treat the schema as
    // constrained.
    for key in obj.keys() {
        let k = key.as_str();
        if METADATA_KEYWORDS.contains(&k) {
            continue;
        }
        // Any other keyword (including all typical constraint keywords like
        // `type`, `properties`, `items`, `enum`, `minimum`, `contains`,
        // `minProperties`, `required`, `additionalProperties`, etc.) is treated
        // as constraining.
        return false;
    }

    // Only metadata keywords were present.
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn opts() -> ConvertOptions {
        ConvertOptions {
            target: Target::OpenaiStrict,
            mode: Mode::Strict,
            ..ConvertOptions::default()
        }
    }

    // ── Root type ──────────────────────────────────────────────
    #[test]
    fn object_root_unchanged() {
        let schema = json!({"type": "object", "properties": {"x": {"type": "string"}}});
        let r = check_provider_compat(schema, &opts());
        assert!(r.pass.transforms.is_empty());
        assert!(r
            .errors
            .iter()
            .all(|e| !matches!(e, ProviderCompatError::RootTypeIncompatible { .. })));
    }

    #[test]
    fn array_root_wrapped() {
        let schema = json!({"type": "array", "items": {"type": "string"}});
        let r = check_provider_compat(schema, &opts());
        assert_eq!(r.pass.transforms.len(), 1);
        assert_eq!(r.pass.schema.get("type").unwrap(), "object");
        assert!(r.pass.schema.pointer("/properties/result/type").unwrap() == "array");
    }

    #[test]
    fn string_root_wrapped() {
        let schema = json!({"type": "string"});
        let r = check_provider_compat(schema, &opts());
        assert_eq!(r.pass.transforms.len(), 1);
        assert!(r.pass.schema.pointer("/properties/result/type").unwrap() == "string");
    }

    #[test]
    fn missing_type_wrapped() {
        let schema = json!({"description": "no type"});
        let r = check_provider_compat(schema, &opts());
        // Root wrap + inner unconstrained → opaque string = 2 transforms
        assert_eq!(r.pass.transforms.len(), 2);
        assert_eq!(r.pass.schema.get("type").unwrap(), "object");
        // The inner schema should be an opaque string now
        let result_schema = &r.pass.schema["properties"]["result"];
        assert_eq!(result_schema["type"], "string");
    }

    // ── #112: type arrays ──────────────────────────────────────
    #[test]
    fn type_array_object_only_no_wrap() {
        // type: "object" (string) should NOT trigger wrapping
        let schema = json!({"type": "object", "properties": {"x": {"type": "string"}}});
        let r = check_provider_compat(schema, &opts());
        assert!(
            r.pass.transforms.is_empty(),
            "exact 'object' root should not be wrapped"
        );
    }

    #[test]
    fn type_array_with_object_null_still_wraps() {
        // type: ["object", "null"] SHOULD trigger wrapping — OpenAI strict requires exactly "object"
        let schema = json!({"type": ["object", "null"], "properties": {"x": {"type": "string"}}});
        let r = check_provider_compat(schema, &opts());
        assert!(
            r.pass
                .transforms
                .iter()
                .any(|t| matches!(t, Transform::RootObjectWrapper { .. })),
            "nullable object root should be wrapped"
        );
        // actual_type should mention both types
        let root_err = r
            .errors
            .iter()
            .find(|e| matches!(e, ProviderCompatError::RootTypeIncompatible { .. }));
        assert!(root_err.is_some(), "should emit RootTypeIncompatible error");
        match root_err.unwrap() {
            ProviderCompatError::RootTypeIncompatible { actual_type, .. } => {
                assert!(
                    actual_type.contains("object"),
                    "actual_type should list 'object'"
                );
                assert!(
                    actual_type.contains("null"),
                    "actual_type should list 'null'"
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn type_array_without_object_wrapped() {
        // type: ["string", "null"] should trigger wrapping with actual_type showing both
        let schema = json!({"type": ["string", "null"]});
        let r = check_provider_compat(schema, &opts());
        assert!(
            r.pass
                .transforms
                .iter()
                .any(|t| matches!(t, Transform::RootObjectWrapper { .. })),
            "non-object type array should be wrapped"
        );
        let root_err = r
            .errors
            .iter()
            .find(|e| matches!(e, ProviderCompatError::RootTypeIncompatible { .. }));
        assert!(root_err.is_some(), "should emit RootTypeIncompatible error");
        match root_err.unwrap() {
            ProviderCompatError::RootTypeIncompatible { actual_type, .. } => {
                assert!(
                    actual_type.contains("string"),
                    "actual_type should list 'string'"
                );
                assert!(
                    actual_type.contains("null"),
                    "actual_type should list 'null'"
                );
            }
            _ => unreachable!(),
        }
    }

    // ── Depth budget ──────────────────────────────────────────
    #[test]
    fn shallow_no_error() {
        let schema = json!({"type": "object", "properties": {"a": {"type": "string"}}});
        let r = check_provider_compat(schema, &opts());
        assert!(r
            .errors
            .iter()
            .all(|e| !matches!(e, ProviderCompatError::DepthBudgetExceeded { .. })));
    }

    #[test]
    fn deep_emits_error() {
        // Build 12 levels deep — exceeds OPENAI_MAX_DEPTH (10)
        let mut inner = json!({"type": "string"});
        for i in (0..12).rev() {
            inner = json!({"type": "object", "properties": {format!("l{i}"): inner}});
        }
        let r = check_provider_compat(inner, &opts());
        let depth_errs: Vec<_> = r
            .errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::DepthBudgetExceeded { .. }))
            .collect();
        assert!(
            !depth_errs.is_empty(),
            "should have at least one depth error"
        );
        // #95: truncation should also produce transforms
        let parse_transforms: Vec<_> = r
            .pass
            .transforms
            .iter()
            .filter(|t| matches!(t, Transform::JsonStringParse { .. }))
            .collect();
        assert!(
            !parse_transforms.is_empty(),
            "deep schema should be truncated with JsonStringParse transforms"
        );
    }

    #[test]
    fn deep_schema_truncated_at_limit() {
        // Build 12 levels deep: root -> l0 -> ... -> l11(string)
        // At OPENAI_MAX_DEPTH (10), the sub-tree should become opaque string
        let mut inner = json!({"type": "string"});
        for i in (0..12).rev() {
            inner = json!({"type": "object", "properties": {format!("l{i}"): inner}});
        }
        let r = check_provider_compat(inner, &opts());

        // The sub-tree at depth >= 10 should be replaced with opaque string
        // Navigate to the deepening path and check for truncation
        let mut cursor = &r.pass.schema;
        for i in 0..10 {
            cursor = &cursor["properties"][format!("l{i}")];
        }
        // At depth 10, the schema should be an opaque string placeholder
        assert_eq!(
            cursor.get("type").and_then(|v| v.as_str()),
            Some("string"),
            "sub-schema at depth limit should be opaque string type"
        );
        assert!(
            cursor.get("description").is_some(),
            "opaque string should have a description"
        );
    }

    #[test]
    fn depth_truncation_preserves_shallow_branches() {
        // One branch 12 deep (exceeds limit), one branch 2 deep (under limit)
        let mut deep = json!({"type": "string"});
        for i in (0..11).rev() {
            deep = json!({"type": "object", "properties": {format!("d{i}"): deep}});
        }
        let schema = json!({
            "type": "object",
            "properties": {
                "deep_branch": deep,
                "shallow": {"type": "string"}
            }
        });
        let r = check_provider_compat(schema, &opts());

        // Shallow branch should be untouched
        assert_eq!(
            r.pass
                .schema
                .pointer("/properties/shallow/type")
                .and_then(|v| v.as_str()),
            Some("string"),
            "shallow branch should remain a string"
        );

        // Deep branch should be truncated somewhere
        let truncate_transforms: Vec<_> = r
            .pass
            .transforms
            .iter()
            .filter(|t| matches!(t, Transform::JsonStringParse { .. }))
            .collect();
        assert!(
            !truncate_transforms.is_empty(),
            "deep branch should produce truncation transforms"
        );
    }

    #[test]
    fn depth_truncation_emits_per_path_errors() {
        // Build two parallel deep branches (12 levels each, exceeds limit of 10)
        let mut deep_a = json!({"type": "string"});
        let mut deep_b = json!({"type": "integer"});
        for i in (0..11).rev() {
            deep_a = json!({"type": "object", "properties": {format!("a{i}"): deep_a}});
            deep_b = json!({"type": "object", "properties": {format!("b{i}"): deep_b}});
        }
        let schema = json!({
            "type": "object",
            "properties": {
                "branch_a": deep_a,
                "branch_b": deep_b,
            }
        });
        let r = check_provider_compat(schema, &opts());

        // Should have multiple per-path depth errors (one for each branch's truncation point)
        let depth_errs: Vec<_> = r
            .errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::DepthBudgetExceeded { .. }))
            .collect();
        assert!(
            depth_errs.len() >= 2,
            "should have at least 2 per-path depth errors, got {}",
            depth_errs.len()
        );
    }

    // ── Enum homogeneity ──────────────────────────────────────
    #[test]
    fn homo_enum_clean() {
        let schema = json!({"type": "object", "properties": {"c": {"enum": ["a", "b"]}}});
        let r = check_provider_compat(schema, &opts());
        assert!(r
            .errors
            .iter()
            .all(|e| !matches!(e, ProviderCompatError::MixedEnumTypes { .. })));
    }

    #[test]
    fn mixed_enum_error() {
        let schema = json!({"type": "object", "properties": {"c": {"enum": ["a", 1]}}});
        let r = check_provider_compat(schema, &opts());
        let enum_errs: Vec<_> = r
            .errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::MixedEnumTypes { .. }))
            .collect();
        assert_eq!(enum_errs.len(), 1);
    }

    // ── Boolean / empty schemas ───────────────────────────────
    #[test]
    fn typed_no_unconstrained() {
        let schema = json!({"type": "object", "properties": {"x": {"type": "string"}}});
        let r = check_provider_compat(schema, &opts());
        assert!(r
            .errors
            .iter()
            .all(|e| !matches!(e, ProviderCompatError::UnconstrainedSchema { .. })));
    }

    #[test]
    fn empty_sub_schema_flagged() {
        let schema = json!({"type": "object", "properties": {"x": {}}});
        let r = check_provider_compat(schema, &opts());
        let uc_errs: Vec<_> = r
            .errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
            .collect();
        assert!(!uc_errs.is_empty());
    }

    // ── Gate: non-OpenAI passthrough ──────────────────────────
    #[test]
    fn gemini_passthrough() {
        let schema = json!({"type": "array"});
        let mut o = opts();
        o.target = Target::Gemini;
        let r = check_provider_compat(schema, &o);
        assert!(r.errors.is_empty());
        assert!(r.pass.transforms.is_empty());
    }

    // ── Boolean false schema ──────────────────────────────────
    #[test]
    fn false_schema_becomes_opaque_string() {
        let schema = json!({"type": "object", "properties": {"deny": false}});
        let r = check_provider_compat(schema, &opts());
        // Should flag as unconstrained
        let uc_errs: Vec<_> = r
            .errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
            .collect();
        assert!(
            !uc_errs.is_empty(),
            "false schema should trigger UnconstrainedSchema"
        );
        // Should produce a JsonStringParse transform
        assert!(
            r.pass
                .transforms
                .iter()
                .any(|t| matches!(t, Transform::JsonStringParse { .. })),
            "false schema should produce JsonStringParse transform"
        );
    }

    // ── Enum collision dedup ──────────────────────────────────
    #[test]
    fn enum_collision_deduplicates() {
        // [1, "1"] should stringify to ["1"] (deduplicated), not ["1", "1"]
        let schema = json!({"type": "object", "properties": {"v": {"enum": [1, "1"]}}});
        let r = check_provider_compat(schema, &opts());
        let enum_vals = r.pass.schema["properties"]["v"]["enum"]
            .as_array()
            .expect("enum should be an array");
        assert_eq!(
            enum_vals.len(),
            1,
            "colliding values should be deduplicated"
        );
        assert_eq!(enum_vals[0], json!("1"));
    }

    // ── #109 Keyword recursion ────────────────────────────────
    #[test]
    fn visitor_strips_pattern_properties_before_recursion() {
        // #246: patternProperties is stripped/stringified before the visitor
        // recurses into children. At root with type:object (no properties),
        // patternProperties is stripped (Branch 3: root).
        let mut schema = json!({
            "type": "object",
            "patternProperties": {
                "^x-": { "enum": ["a", 1] }
            }
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        // patternProperties should be stripped at root
        assert!(
            schema.get("patternProperties").is_none(),
            "patternProperties should be stripped at root"
        );
        // Should emit a PatternPropertiesStripped error (not MixedEnumTypes)
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ProviderCompatError::PatternPropertiesStripped { .. })),
            "should emit PatternPropertiesStripped error"
        );
    }

    #[test]
    fn visitor_recurses_dependent_schemas() {
        // dependentSchemas values should be visited for unconstrained detection
        let mut schema = json!({
            "type": "object",
            "dependentSchemas": {
                "foo": {}
            }
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        let uc_errs: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
            .collect();
        assert!(
            !uc_errs.is_empty(),
            "empty schema inside dependentSchemas should trigger UnconstrainedSchema"
        );
    }

    // ── #111 Depth semantics ──────────────────────────────────
    #[test]
    fn visitor_combinator_no_depth_increment() {
        // anyOf/oneOf/allOf should NOT increment semantic depth (max_depth_observed)
        let mut schema = json!({
            "type": "object",
            "anyOf": [{
                "type": "object",
                "oneOf": [{
                    "type": "string"
                }]
            }]
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        assert_eq!(
            visitor.max_depth_observed, 0,
            "combinators should not increment semantic depth, got: {}",
            visitor.max_depth_observed
        );
    }

    #[test]
    fn visitor_data_shape_keywords_increment_depth() {
        // properties/items/additionalProperties should increment semantic depth
        let mut schema = json!({
            "type": "object",
            "properties": {
                "child": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    }
                }
            }
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        assert_eq!(
            visitor.max_depth_observed, 2,
            "properties → items should yield semantic depth 2, got: {}",
            visitor.max_depth_observed
        );
    }

    // ── Tuple items traversal (Gemini review finding) ────────
    #[test]
    fn visitor_recurses_tuple_items() {
        // Array-form items (tuple validation) should be traversed for mixed enum detection
        let mut schema = json!({
            "type": "array",
            "items": [
                { "type": "string" },
                { "enum": ["a", 1] }
            ]
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        let enum_errs: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::MixedEnumTypes { .. }))
            .collect();
        assert!(
            !enum_errs.is_empty(),
            "mixed enum inside tuple items should be detected"
        );
    }

    // ── Boolean schema consistency ───────────────────────────
    #[test]
    fn visitor_boolean_true_detected_across_data_shape_keywords() {
        // `true` booleans should be caught as unconstrained across all data-shape keywords
        for keyword in &[
            "items",
            "additionalProperties",
            "unevaluatedProperties",
            "unevaluatedItems",
            "contains",
        ] {
            let mut schema = json!({
                "type": "object",
                (keyword.to_string()): true
            });
            let mut errors = Vec::new();
            let mut transforms = Vec::new();
            let mut visitor = CompatVisitor {
                errors: &mut errors,
                transforms: &mut transforms,
                target: Target::OpenaiStrict,
                max_depth_observed: 0,
            };
            visitor.visit(&mut schema, "#", 0, 0);
            let unconstrained: Vec<_> = errors
                .iter()
                .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
                .collect();
            assert!(
                !unconstrained.is_empty(),
                "'{}: true' should be detected as unconstrained",
                keyword
            );
        }
    }

    #[test]
    fn visitor_boolean_false_left_alone_across_data_shape_keywords() {
        // `false` booleans are sealed constraints — should NOT trigger UnconstrainedSchema
        for keyword in &[
            "items",
            "additionalProperties",
            "unevaluatedProperties",
            "unevaluatedItems",
            "contains",
        ] {
            let mut schema = json!({
                "type": "object",
                (keyword.to_string()): false
            });
            let mut errors = Vec::new();
            let mut transforms = Vec::new();
            let mut visitor = CompatVisitor {
                errors: &mut errors,
                transforms: &mut transforms,
                target: Target::OpenaiStrict,
                max_depth_observed: 0,
            };
            visitor.visit(&mut schema, "#", 0, 0);
            let unconstrained: Vec<_> = errors
                .iter()
                .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
                .collect();
            assert!(
                unconstrained.is_empty(),
                "'{}: false' should NOT be detected as unconstrained, but got {:?}",
                keyword,
                unconstrained
            );
        }
    }

    // ── Gap #1: additionalProperties: true nested inside properties ──
    #[test]
    fn visitor_nested_additional_properties_true_caught() {
        // A property with `additionalProperties: true` should be detected as
        // unconstrained, even though the root wrapper injects `false` at top level.
        let mut schema = json!({
            "type": "object",
            "properties": {
                "meta": {
                    "type": "object",
                    "additionalProperties": true
                }
            }
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        let unconstrained: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
            .collect();
        assert!(
            !unconstrained.is_empty(),
            "nested additionalProperties: true should be caught"
        );
        // Verify the path points inside the nested property, not the root
        let paths: Vec<_> = errors
            .iter()
            .filter_map(|e| match e {
                ProviderCompatError::UnconstrainedSchema { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            paths.iter().any(|p| p.contains("meta")),
            "error path should reference the nested property, got {:?}",
            paths
        );
    }

    // ── Gap #2: Root wrapper additionalProperties: false is NOT transformed ──
    #[test]
    fn visitor_root_wrapper_additional_properties_false_preserved() {
        // The root wrapper adds `additionalProperties: false`. The visitor
        // must NOT flag this as unconstrained or transform it.
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "additionalProperties": false,
            "required": ["name"]
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        // The `false` should remain untouched — no UnconstrainedSchema error for it
        let unconstrained: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
            .collect();
        assert!(
            unconstrained.is_empty(),
            "additionalProperties: false should NOT be flagged, got {:?}",
            unconstrained
        );
        // Verify the original value is still false (not transformed to opaque string)
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&json!(false)),
            "additionalProperties: false should be preserved, not transformed"
        );
    }

    // ── Gap #3: Errors inside if/then/else are reported ──
    #[test]
    fn visitor_recurses_into_if_then_else() {
        // An unconstrained schema inside `then` should be detected
        let mut schema = json!({
            "type": "object",
            "if": { "properties": { "kind": { "const": "a" } } },
            "then": {
                "type": "object",
                "properties": {
                    "nested": {}
                }
            },
            "else": {
                "type": "object",
                "properties": {
                    "fallback": {}
                }
            }
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        let unconstrained_paths: Vec<_> = errors
            .iter()
            .filter_map(|e| match e {
                ProviderCompatError::UnconstrainedSchema { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        // Should find unconstrained schemas inside both then and else
        assert!(
            unconstrained_paths
                .iter()
                .any(|p| p.contains("then") && p.contains("nested")),
            "unconstrained schema inside 'then' should be detected, got {:?}",
            unconstrained_paths
        );
        assert!(
            unconstrained_paths
                .iter()
                .any(|p| p.contains("else") && p.contains("fallback")),
            "unconstrained schema inside 'else' should be detected, got {:?}",
            unconstrained_paths
        );
    }

    // ── Gap #4: contains with empty schema detected ──
    #[test]
    fn visitor_recurses_into_contains() {
        // An empty `contains` schema should be detected as unconstrained
        let mut schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "contains": {}
                }
            }
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        let unconstrained_paths: Vec<_> = errors
            .iter()
            .filter_map(|e| match e {
                ProviderCompatError::UnconstrainedSchema { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            unconstrained_paths.iter().any(|p| p.contains("contains")),
            "unconstrained 'contains' schema should be detected, got {:?}",
            unconstrained_paths
        );
    }

    // ── Gap #5: Semantic depth through combinator→data-shape chain ──
    #[test]
    fn visitor_depth_through_combinator_then_data_shape() {
        // anyOf → items should increment semantic depth only for items, not anyOf
        // Expected: root(0) → anyOf(0) → items(1) = max semantic depth 1
        let mut schema = json!({
            "type": "object",
            "properties": {
                "field": {
                    "anyOf": [
                        {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "val": { "type": "string" }
                                }
                            }
                        }
                    ]
                }
            }
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        // properties(1) → anyOf(1, combinator, no increment) → items(2) → properties(3)
        assert_eq!(
            visitor.max_depth_observed, 3,
            "properties → anyOf → items → properties should yield semantic depth 3, got: {}",
            visitor.max_depth_observed
        );
    }

    // ── #122: Mixed-type prefixItems + conflicting items ───────────────

    #[test]
    fn test_mixed_prefixitems_conflicting_items_replaced_with_union() {
        // prefixItems has mixed types (integer, boolean, string) but items says "string".
        // items should be replaced with anyOf union of all distinct types.
        let schema = json!({
            "type": "array",
            "prefixItems": [
                {"type": "integer"},
                {"type": "boolean"},
                {"type": "string"}
            ],
            "items": {"type": "string"},
            "description": "Mixed tuple"
        });
        let r = check_provider_compat(schema, &opts());
        let inner = &r.pass.schema["properties"]["result"];
        // items should be present (OpenAI requires it) but as anyOf union
        let items = inner
            .get("items")
            .expect("items must be preserved for OpenAI strict mode");
        let any_of = items.get("anyOf").expect("items should be anyOf union");
        let types: Vec<&str> = any_of
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.get("type").and_then(|t| t.as_str()))
            .collect();
        // BTreeSet ordering: boolean, integer, string
        assert_eq!(types, vec!["boolean", "integer", "string"]);
        assert!(
            inner.get("prefixItems").is_some(),
            "prefixItems should be preserved"
        );
    }

    #[test]
    fn test_homogeneous_prefixitems_items_preserved() {
        // prefixItems is all "string", items is also "string" — no conflict.
        let schema = json!({
            "type": "array",
            "prefixItems": [
                {"type": "string"},
                {"type": "string"}
            ],
            "items": {"type": "string"},
            "description": "Homogeneous tuple"
        });
        let r = check_provider_compat(schema, &opts());
        let inner = &r.pass.schema["properties"]["result"];
        assert!(
            inner.get("items").is_some(),
            "items should be preserved when prefixItems types all match items type"
        );
    }

    #[test]
    fn test_no_prefixitems_items_preserved() {
        // Plain array with items only — no prefixItems, no conflict.
        let schema = json!({
            "type": "array",
            "items": {"type": "string"},
            "description": "Simple array"
        });
        let r = check_provider_compat(schema, &opts());
        let inner = &r.pass.schema["properties"]["result"];
        assert!(
            inner.get("items").is_some(),
            "items should be preserved when there are no prefixItems"
        );
    }

    // ── #246: patternProperties handling ──────────────────────────

    #[test]
    fn pattern_properties_typed_with_props_stripped() {
        // Typed object with both properties AND patternProperties →
        // strip patternProperties, keep structure intact.
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "patternProperties": {
                "^x-": { "type": "string" }
            },
            "additionalProperties": false,
            "required": ["name"]
        });
        let r = check_provider_compat(schema, &opts());

        // Root should still be an object with properties
        assert_eq!(r.pass.schema["type"], "object");
        assert!(r.pass.schema["properties"]["name"].is_object());
        // patternProperties should be stripped
        assert!(
            r.pass.schema.get("patternProperties").is_none(),
            "patternProperties should be stripped from typed object with properties"
        );
        // Should emit PatternPropertiesStripped error
        assert!(
            r.errors
                .iter()
                .any(|e| matches!(e, ProviderCompatError::PatternPropertiesStripped { .. })),
            "should emit PatternPropertiesStripped error"
        );
    }

    #[test]
    fn pattern_properties_typed_sterile_stringified() {
        // Typed object with patternProperties but NO properties →
        // opaque-stringify (stripping would leave useless {}).
        // Wrap in a parent so it's not root.
        let schema = json!({
            "type": "object",
            "properties": {
                "ext": {
                    "type": "object",
                    "patternProperties": {
                        "^x-": {}
                    },
                    "additionalProperties": false
                }
            },
            "additionalProperties": false,
            "required": ["ext"]
        });
        let r = check_provider_compat(schema, &opts());

        // Inner "ext" should be opaque-stringified
        assert_eq!(
            r.pass.schema["properties"]["ext"]["type"], "string",
            "sterile typed object should be opaque-stringified"
        );
        // Should have a JsonStringParse transform for the ext path
        assert!(
            r.pass.transforms.iter().any(|t| matches!(
                t,
                Transform::JsonStringParse { path } if path.contains("ext")
            )),
            "should emit JsonStringParse transform for sterile object"
        );
    }

    #[test]
    fn pattern_properties_untyped_stringified() {
        // Untyped schema with only patternProperties (nested, not root) →
        // opaque-stringify.
        let schema = json!({
            "type": "object",
            "properties": {
                "value": {
                    "patternProperties": {
                        "^x-": { "type": "string" }
                    }
                }
            },
            "additionalProperties": false,
            "required": ["value"]
        });
        let r = check_provider_compat(schema, &opts());

        // Inner "value" should be opaque-stringified
        assert_eq!(
            r.pass.schema["properties"]["value"]["type"], "string",
            "untyped patternProperties-only schema should be opaque-stringified"
        );
        assert!(
            r.pass.transforms.iter().any(|t| matches!(
                t,
                Transform::JsonStringParse { path } if path.contains("value")
            )),
            "should emit JsonStringParse transform"
        );
    }

    #[test]
    fn pattern_properties_nested_in_anyof() {
        // patternProperties inside anyOf variant → opaque-stringify.
        let schema = json!({
            "type": "object",
            "properties": {
                "field": {
                    "anyOf": [
                        {
                            "patternProperties": {
                                "^x-": {}
                            },
                            "description": "Extension object"
                        },
                        { "type": "null" }
                    ]
                }
            },
            "additionalProperties": false,
            "required": ["field"]
        });
        let r = check_provider_compat(schema, &opts());

        // The anyOf[0] (patternProperties variant) should be opaque-stringified
        let anyof = r.pass.schema["properties"]["field"]["anyOf"]
            .as_array()
            .expect("anyOf should exist");
        let pp_variant = &anyof[0];
        assert_eq!(
            pp_variant["type"], "string",
            "patternProperties variant in anyOf should be opaque-stringified"
        );
        assert!(
            pp_variant.get("patternProperties").is_none(),
            "patternProperties should not remain after stringification"
        );
    }

    #[test]
    fn pattern_properties_root_stripped() {
        // Root with patternProperties but no type → check_root_type wraps it,
        // then visitor should strip patternProperties from the inner schema.
        let schema = json!({
            "patternProperties": {
                "^x-": { "type": "string" }
            },
            "properties": {
                "name": { "type": "string" }
            }
        });
        let r = check_provider_compat(schema, &opts());

        // Root should be wrapped (no explicit type)
        assert_eq!(r.pass.schema["type"], "object");
        // patternProperties should not survive anywhere in the output
        fn has_pattern_props(v: &Value) -> bool {
            match v {
                Value::Object(obj) => {
                    obj.contains_key("patternProperties") || obj.values().any(has_pattern_props)
                }
                Value::Array(arr) => arr.iter().any(has_pattern_props),
                _ => false,
            }
        }
        assert!(
            !has_pattern_props(&r.pass.schema),
            "patternProperties should not survive in the output schema"
        );
    }
}
