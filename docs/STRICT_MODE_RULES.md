# Strict Mode Rules Reference

OpenAI's [Structured Outputs](https://platform.openai.com/docs/guides/structured-outputs) feature ("strict mode") imposes a restrictive subset of JSON Schema. Schemas that violate any rule are rejected at request time with an API error.

This document catalogs every known restriction, maps it to the pipeline pass that handles it, the `validate_strict_mode` check that catches violations, and explains **why** the restriction exists.

## Quick Reference

| ID    | Rule                                            | Pipeline Pass        | Validator Check               | Why                                                                                  |
| ----- | ----------------------------------------------- | -------------------- | ----------------------------- | ------------------------------------------------------------------------------------ |
| SM-01 | Root must be `type: "object"`                   | p9 `check_root_type` | `RootNotObject`               | OpenAI wraps the root in a function call; only objects are valid argument containers |
| SM-02 | All schema nodes must declare `type`            | p9 type inference    | `MissingType`                 | The parser needs unambiguous type dispatch for every node                            |
| SM-03 | Objects must have `additionalProperties: false` | p6 `enforce_strict`  | `MissingAdditionalProperties` | Prevents the model from hallucinating extra fields outside the schema                |
| SM-04 | Arrays must have `items`                        | p9 visitor           | `MissingItems`                | Unbounded arrays can't be validated; the parser needs an element schema              |
| SM-05 | No `patternProperties`                          | p9 strip/stringify   | `BannedPatternProperties`     | Regex-keyed maps are incompatible with the deterministic output parser               |
| SM-06 | No `$ref`                                       | p5 ref resolution    | `BannedSchemaRef`             | All references must be resolved inline; the API does not resolve `$ref`              |
| SM-07 | No `$anchor`                                    | p9 strip             | `BannedAnchor`                | Fragment identifiers are not supported by the runtime validator                      |
| SM-08 | No `$dynamicRef`                                | p9 strip             | `BannedDynamicRef`            | Dynamic scoping is not supported                                                     |
| SM-09 | No `$dynamicAnchor`                             | p9 strip             | `BannedDynamicAnchor`         | Dynamic scoping is not supported                                                     |
| SM-10 | No `dependentSchemas`                           | p9 strip             | `BannedDependentSchemas`      | Conditional sub-schemas add non-deterministic validation paths                       |
| SM-11 | No `dependentRequired`                          | p9 strip             | `BannedDependentRequired`     | Conditional required fields add non-deterministic validation paths                   |
| SM-12 | No `unevaluatedProperties`                      | p9 strip             | `BannedUnevaluatedProperties` | Cross-schema annotation collection is not implemented                                |
| SM-13 | No `unevaluatedItems`                           | p9 strip             | `BannedUnevaluatedItems`      | Cross-schema annotation collection is not implemented                                |
| SM-14 | No `contains`                                   | p9 strip             | `BannedContains`              | Array containment checks are not supported by the output parser                      |
| SM-15 | No `minContains`                                | p9 strip             | `BannedMinContains`           | Companion to `contains`; not supported                                               |
| SM-16 | No `maxContains`                                | p9 strip             | `BannedMaxContains`           | Companion to `contains`; not supported                                               |
| SM-17 | No `if`                                         | p9 strip             | `BannedIf`                    | Conditional schemas create non-deterministic output paths                            |
| SM-18 | No `then`                                       | p9 strip             | `BannedThen`                  | Companion to `if`; not supported                                                     |
| SM-19 | No `else`                                       | p9 strip             | `BannedElse`                  | Companion to `if`; not supported                                                     |
| SM-20 | No `not`                                        | p9 strip             | `BannedNot`                   | Negation constraints are incompatible with the generative output model               |
| SM-21 | Semantic depth ≤ 5                              | p9 depth truncation  | `DepthExceeded`               | OpenAI's parser has a fixed nesting limit of 5 data-shape levels                     |

## Rule Details

### SM-01: Root Type Object

The root schema **must** be `type: "object"`. OpenAI wraps the schema in a function-call tool definition where the root represents the function's argument object. Non-object roots (arrays, strings, etc.) are wrapped by p9's `check_root_type` into `{ type: "object", properties: { result: <original> } }`.

### SM-02: Explicit Type Declaration

Every schema node must declare `type`. Exceptions: bare combinator wrappers (`anyOf`/`oneOf`/`allOf` without `type`) and schemas with `enum`/`const` (where type is implicit). The parser uses `type` for dispatch — without it, the node is ambiguous.

### SM-03: Closed Objects

All `type: "object"` schemas must have `additionalProperties: false`. This is the core strict-mode invariant — it forces the model to produce **only** the declared properties, preventing hallucinated extra fields. Handled by p6 which also makes all properties `required` and wraps optional ones in `anyOf: [T, {type: "null"}]`.

### SM-04: Typed Arrays

All `type: "array"` schemas must have `items` (or `prefixItems`). Without an element schema, the parser cannot validate array contents.

### SM-05–SM-09: Reference & Anchor Keywords

`$ref`, `$anchor`, `$dynamicRef`, `$dynamicAnchor`, and `patternProperties` are all banned. The API does not implement JSON Schema's reference resolution, fragment identification, or dynamic scoping. The pipeline resolves all `$ref`s inline (p5) and strips the rest (p9).

### SM-10–SM-11: Dependency Keywords

`dependentSchemas` and `dependentRequired` create conditional validation paths where the applicable schema changes based on which properties are present. The deterministic output parser cannot handle this branching.

### SM-12–SM-13: Unevaluated Keywords

`unevaluatedProperties` and `unevaluatedItems` require cross-schema annotation collection (tracking which properties/items were "evaluated" by sibling schemas). This Draft 2020-12 feature is not implemented in the API's validator.

### SM-14–SM-16: Containment Keywords

`contains`, `minContains`, and `maxContains` assert that an array includes at least one item matching a sub-schema. The output parser generates arrays sequentially and cannot retroactively guarantee containment.

### SM-17–SM-19: Conditional Keywords

`if`/`then`/`else` create conditional schema branches. The generative model needs a single deterministic output shape, not conditional paths that depend on the values being generated.

### SM-20: Negation

`not` defines schemas by exclusion. The generative model works by producing values that _match_ a schema, not by avoiding values that match a negated one.

### SM-21: Depth Limit

Semantic nesting depth must not exceed 5. "Semantic depth" counts data-shape edges: `properties`, `items`, `additionalProperties`, `prefixItems`. Combinators (`anyOf`/`oneOf`/`allOf`) do **not** increment depth. Schemas exceeding the limit are truncated to opaque strings by p9.

## Allowed Keywords

For reference, these JSON Schema keywords **are** supported in strict mode:

- **Structural**: `type`, `properties`, `items`, `prefixItems`, `additionalProperties`, `required`
- **Composition**: `anyOf`, `oneOf`, `allOf`
- **Validation**: `enum`, `const`
- **Metadata**: `title`, `description`, `default`, `examples`
- **Identity**: `$id`, `$defs`
- **String**: `minLength`, `maxLength`, `pattern`
- **Numeric**: `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf`
- **Array**: `minItems`, `maxItems`, `uniqueItems`
