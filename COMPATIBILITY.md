# JSON Schema Compatibility Matrix (Alpha)

This document tracks the support status of JSON Schema features in `jsonschema-llm`. The library targets **JSON Schema Draft 2020-12**.

**Status Legend:**

- ✅ **Supported**: Fully handled by the compiler pipeline
- ⚠️ **Partial**: Works for common cases; known limitations exist
- ❌ **Unsupported**: Not yet implemented; causes errors or ignored

---

## Core Keywords

| Keyword                 | Status | Notes                                                             |
| :---------------------- | :----- | :---------------------------------------------------------------- |
| `$ref`                  | ✅     | Full support for local, remote, and file-system refs.             |
| `$defs` / `definitions` | ✅     | Fully supported.                                                  |
| `type`                  | ✅     | Single types and arrays (e.g. `["string", "null"]`).              |
| `enum`                  | ✅     | String/integer enums supported. Mixed enums handled via coercion. |
| `const`                 | ✅     | Treated as single-value enum.                                     |

## Object Keywords

| Keyword                           | Status | Notes                                                                           |
| :-------------------------------- | :----- | :------------------------------------------------------------------------------ |
| `properties`                      | ✅     |                                                                                 |
| `required`                        | ✅     |                                                                                 |
| `additionalProperties`            | ✅     | `true`/`{}`/schema supported. Converted to strict object or extra field bucket. |
| `patternProperties`               | ⚠️     | Compilation succeeds, but validation is best-effort during rehydration.         |
| `unevaluatedProperties`           | ❌     | Ignored (treated as permissive).                                                |
| `propertyNames`                   | ❌     | Ignored.                                                                        |
| `minProperties` / `maxProperties` | ❌     | Ignored.                                                                        |
| `dependentRequired`               | ❌     | Ignored.                                                                        |
| `dependentSchemas`                | ❌     | Ignored.                                                                        |

## Array Keywords

| Keyword                 | Status | Notes                                                             |
| :---------------------- | :----- | :---------------------------------------------------------------- |
| `items`                 | ✅     |                                                                   |
| `prefixItems`           | ✅     | Tuple support via array flattening or `oneOf` unrolling.          |
| `uniqueItems`           | ❌     | Ignored.                                                          |
| `minItems` / `maxItems` | ⚠️     | `maxItems` enforced (truncate). `minItems` advisory warning only. |
| `contains`              | ❌     | Ignored.                                                          |
| `unevaluatedItems`      | ❌     | Ignored.                                                          |

## Composition & Polymorphism

| Keyword                | Status | Notes                                                                          |
| :--------------------- | :----- | :----------------------------------------------------------------------------- |
| `allOf`                | ✅     | Merged at compile time.                                                        |
| `oneOf`                | ✅     | Converted to `anyOf` for LLM compatibility.                                    |
| `anyOf`                | ✅     | Supported.                                                                     |
| `not`                  | ❌     | Ignored.                                                                       |
| `if` / `then` / `else` | ⚠️     | Dropped during compilation; validated as advisory warnings during rehydration. |
| `discriminator`        | ⚠️     | Hint used for `oneOf` resolution, but validation is loose.                     |

## String Constraints

| Keyword                   | Status | Notes                                                               |
| :------------------------ | :----- | :------------------------------------------------------------------ |
| `minLength` / `maxLength` | ⚠️     | `maxLength` enforced (truncate). `minLength` advisory warning only. |
| `pattern`                 | ✅     | Validated during rehydration (advisory warning on mismatch).        |
| `format`                  | ❌     | Ignored (most LLMs don't support format validation natively).       |

## Numeric Constraints

| Keyword                                 | Status | Notes                                   |
| :-------------------------------------- | :----- | :-------------------------------------- |
| `minimum` / `maximum`                   | ✅     | Enforced during rehydration (clamping). |
| `exclusiveMinimum` / `exclusiveMaximum` | ✅     | Enforced during rehydration (clamping). |
| `multipleOf`                            | ❌     | Ignored.                                |

---

## Known Limitations

1. **Recursion**: Recursive schemas are supported up to a configurable depth (default: 3). Deeply nested recursive structures may be truncated.
2. **Mixed-Type Arrays**: Arrays with mixed types (e.g. `[1, "string"]`) may have reduced fidelity in some LLM providers.
3. **Opaque Objects**: Schemas using `type: "object"` without properties are converted to JSON strings to avoid hallucination, requiring rehydration to restore.
