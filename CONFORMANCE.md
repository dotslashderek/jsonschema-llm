# Conformance Report

Pipeline conformance against the [JSON Schema Test Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite).

> **Note**: This pipeline is a _compiler_, not a _validator_. We test that every schema in the
> suite survives the full multi-pass pipeline without panicking and produces valid output.
> The `data`/`valid` fields in each test case are not applicable.

### Terminology

| Term             | Meaning                                                                       |
| ---------------- | ----------------------------------------------------------------------------- |
| **Pass**         | `convert()` returned `Ok(result)` with a valid JSON schema                    |
| **Graceful-err** | `convert()` returned a well-typed `Err(ConvertError)` — intentional rejection |
| **Fail**         | Panic or codec serialization error — always a bug                             |

## Draft 2020-12

| Metric                  | Value    |
| ----------------------- | -------- |
| Keyword files tested    | 43       |
| Skipped                 | 3        |
| Pass (compiled Ok)      | 43 files |
| Graceful-err            | 0        |
| Failed                  | 0        |
| **Effective pass rate** | **100%** |

### Skipped Files

| File              | Reason                                              |
| ----------------- | --------------------------------------------------- |
| `dynamicRef.json` | `$dynamicRef`/`$dynamicAnchor` — not yet supported  |
| `refRemote.json`  | Requires HTTP remote `$ref` resolution              |
| `vocabulary.json` | Meta-schema vocabulary negotiation — not applicable |

## Draft 2019-09

Not yet tested. Planned for future scope.

## Draft 7

Not yet tested. Planned for future scope.
