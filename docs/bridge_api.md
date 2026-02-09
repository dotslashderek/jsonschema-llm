# Bridge API Response Contract

This document describes the JSON shape of responses from the FFI bridge functions
(`convert_json`, `rehydrate_json`). These are the entry points used by language
bindings (WASM/TypeScript, Python, Java, etc.).

## Version Contract

- **Output**: Every successful response includes `"apiVersion": "1.0"`.
- **Input**: No version field required on requests (single version, forward-compatible).
- **Codec**: Uses `$schema` URI for format versioning (`https://jsonschema-llm.dev/codec/v1`).

When input versioning becomes necessary, a `version` field will be added to `ConvertOptions`.

## Convert Response

```json
{
  "apiVersion": "1.0",
  "schema": { "..." },
  "codec": {
    "$schema": "https://jsonschema-llm.dev/codec/v1",
    "transforms": [],
    "droppedConstraints": []
  }
}
```

## Rehydrate Response

```json
{
  "apiVersion": "1.0",
  "data": { "..." },
  "warnings": [
    {
      "dataPath": "/users/0/email",
      "schemaPath": "#/properties/users/items/properties/email",
      "kind": { "type": "constraint_violation", "constraint": "pattern" },
      "message": "..."
    }
  ]
}
```

## Error Response

Returned as the `Err` variant (JSON string):

```json
{
  "code": "schema_error",
  "message": "Schema error at #/properties/foo: unsupported keyword",
  "path": "#/properties/foo"
}
```

### Error Codes

| Code                       | Description                                          |
| -------------------------- | ---------------------------------------------------- |
| `json_parse_error`         | Malformed JSON input                                 |
| `schema_error`             | Invalid or unsupported schema construct              |
| `recursion_depth_exceeded` | Max `$ref` traversal depth exceeded                  |
| `unsupported_feature`      | Feature not supported by target provider             |
| `unresolvable_ref`         | `$ref` cannot be resolved                            |
| `rehydration_error`        | Error during data rehydration                        |
| `codec_version_mismatch`   | Codec `$schema` version incompatible with this build |

## Codec Version Validation

The rehydrator validates the codec `$schema` URI before applying transforms:

- **URI format**: `https://jsonschema-llm.dev/codec/v{major}`
- **Hard-fail**: If major version ≠ expected (currently `1`)
- **Error**: `CodecVersionMismatch` with `found` and `expected` fields
- **Malformed URI**: Also produces `CodecVersionMismatch`

## Field Naming Convention

| Type family             | Convention   | Example                             |
| ----------------------- | ------------ | ----------------------------------- |
| Output DTOs (bridge)    | `camelCase`  | `apiVersion`, `droppedConstraints`  |
| Codec transforms (tags) | `snake_case` | `map_to_array`, `json_string_parse` |
| Input options           | `kebab-case` | `max-depth`, `recursion-limit`      |
| Error codes             | `snake_case` | `codec_version_mismatch`            |
| Warning kinds (tags)    | `snake_case` | `constraint_violation`              |

## DTO Evolution

Core Rust types (`ConvertResult`, `RehydrateResult`) remain clean — `apiVersion` is
injected by bridge wrapper structs. If a second binding target requires DTO divergence
or field-level breaking changes arise, types may be extracted to a dedicated module.
