/**
 * TypeScript type-checking test for jsonschema-llm WASM bindings.
 *
 * This file is compiled with `tsc --noEmit` to verify that the hand-authored
 * TypeScript definitions in the WASM package accurately reflect the runtime API.
 *
 * It is NOT executed at runtime — only type-checked.
 */
import {
  convert,
  rehydrate,
  type ConvertOptions,
  type ConvertResult,
  type Codec,
  type RehydrateResult,
  type Warning,
  type WarningKind,
  type Transform,
  type DroppedConstraint,
  type ErrorCode,
  type StructuredError,
  type Target,
  type PolymorphismStrategy,
} from "jsonschema-llm-wasm";

// ---------------------------------------------------------------------------
// convert — basic usage
// ---------------------------------------------------------------------------

const result: ConvertResult = convert({ type: "object" });

// Boolean schemas are valid JSON Schema
const _boolTrue: ConvertResult = convert(true);
const _boolFalse: ConvertResult = convert(false);

// apiVersion is a string
const _version: string = result.apiVersion;

// schema is a Record
const _schema: Record<string, unknown> = result.schema;

// ---------------------------------------------------------------------------
// convert — options handling (camelCase only)
// ---------------------------------------------------------------------------

const withOpts: ConvertResult = convert(
  { type: "string" },
  { target: "gemini", maxDepth: 10, recursionLimit: 2 },
);

// All option fields are optional
const minimal: ConvertResult = convert({ type: "string" }, {});

// null and undefined options accepted
const withNull: ConvertResult = convert({ type: "string" }, null);
const withUndef: ConvertResult = convert({ type: "string" });

// ---------------------------------------------------------------------------
// Target and PolymorphismStrategy enums
// ---------------------------------------------------------------------------

const _t1: Target = "openai-strict";
const _t2: Target = "gemini";
const _t3: Target = "claude";
const _p1: PolymorphismStrategy = "any-of";
const _p2: PolymorphismStrategy = "flatten";

// Full options object
const _opts: ConvertOptions = {
  target: "openai-strict",
  maxDepth: 50,
  recursionLimit: 3,
  polymorphism: "any-of",
};

// ---------------------------------------------------------------------------
// Codec shape narrowing
// ---------------------------------------------------------------------------

const codec: Codec = result.codec;
const _$schema: string = codec.$schema;
const transforms: Transform[] = codec.transforms;
const dropped: DroppedConstraint[] = codec.droppedConstraints;

// DroppedConstraint fields
if (dropped.length > 0) {
  const dc = dropped[0];
  const _dcPath: string = dc.path;
  const _dcConstraint: string = dc.constraint;
  const _dcValue: unknown = dc.value;
}

// Transform discriminated union narrowing
if (transforms.length > 0) {
  const t = transforms[0];
  switch (t.type) {
    case "map_to_array":
      const _kf: string = t.keyField;
      break;
    case "json_string_parse":
      const _jp: string = t.path;
      break;
    case "nullable_optional":
      const _or: boolean = t.originalRequired;
      break;
    case "discriminator_any_of":
      const _disc: string = t.discriminator;
      const _vars: string[] = t.variants;
      break;
    case "extract_additional_properties":
      const _pn: string = t.propertyName;
      break;
    case "recursive_inflate":
      const _oref: string = t.originalRef;
      break;
    default: {
      // Exhaustive check — fails at compile time if a new variant is added
      const _exhaustive: never = t;
      break;
    }
  }
}

// ---------------------------------------------------------------------------
// rehydrate — round-trip
// ---------------------------------------------------------------------------

const rehydrated: RehydrateResult = rehydrate({ name: "test" }, result.codec);

const _rehydratedVersion: string = rehydrated.apiVersion;
const _rehydratedData: unknown = rehydrated.data;
const warnings: Warning[] = rehydrated.warnings;

// Warning shape
if (warnings.length > 0) {
  const w = warnings[0];
  const _dp: string = w.dataPath;
  const _sp: string = w.schemaPath;
  const _msg: string = w.message;
  const kind: WarningKind = w.kind;

  // WarningKind discriminated union
  switch (kind.type) {
    case "constraint_violation":
      const _cv: string = kind.constraint;
      break;
    case "constraint_unevaluable":
      const _cu: string = kind.constraint;
      break;
    case "path_not_found":
      // No extra fields
      break;
    default: {
      // Exhaustive check — fails at compile time if a new variant is added
      const _exhaustive: never = kind;
      break;
    }
  }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

const _ec1: ErrorCode = "json_parse_error";
const _ec2: ErrorCode = "schema_error";
const _ec3: ErrorCode = "recursion_depth_exceeded";
const _ec4: ErrorCode = "unsupported_feature";
const _ec5: ErrorCode = "unresolvable_ref";
const _ec6: ErrorCode = "rehydration_error";
const _ec7: ErrorCode = "codec_version_mismatch";

const _err: StructuredError = {
  code: "json_parse_error",
  message: "test error",
  path: null,
};

const _errWithPath: StructuredError = {
  code: "schema_error",
  message: "test error",
  path: "/properties/name",
};
