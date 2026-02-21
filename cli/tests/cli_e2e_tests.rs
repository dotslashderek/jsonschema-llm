//! CLI end-to-end tests that exercise the CLI binary against fixture schemas.
//! These complement the existing `cli_tests.rs` by using shared fixture files.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/schemas");

#[allow(deprecated)]
fn cmd() -> Command {
    Command::cargo_bin("jsonschema-llm").expect("binary should exist")
}

/// Fixtures that convert cleanly with zero provider compat errors.
fn clean_fixtures() -> Vec<&'static str> {
    vec![
        "simple",
        "discriminator",
        "opaque",
        "allof",
        "maps",
        "recursive",
    ]
}

/// Fixtures that trigger advisory provider compat diagnostics (e.g. depth budget)
/// but still produce valid output. The CLI exits 0 (transforms were applied).
fn warned_fixtures() -> Vec<&'static str> {
    vec!["kitchen_sink", "deep_objects"]
}

// ── E2E: Convert all fixtures via CLI ───────────────────────────────────────

#[test]
fn test_cli_e2e_convert_all_fixtures() {
    let dir = TempDir::new().unwrap();

    // Helper: validate output files exist and contain valid JSON
    let validate_outputs = |name: &str, output: &std::path::Path, codec: &std::path::Path| {
        let out_content = fs::read_to_string(output)
            .unwrap_or_else(|e| panic!("Output file for {name} missing: {e}"));
        let _: serde_json::Value =
            serde_json::from_str(&out_content).expect("output should be valid JSON");

        let codec_content = fs::read_to_string(codec)
            .unwrap_or_else(|e| panic!("Codec file for {name} missing: {e}"));
        let _: serde_json::Value =
            serde_json::from_str(&codec_content).expect("codec should be valid JSON");
    };

    // Clean fixtures: should exit 0
    for name in clean_fixtures() {
        let input = format!("{FIXTURES_DIR}/{name}.json");
        let output = dir.path().join(format!("{name}.converted.json"));
        let codec = dir.path().join(format!("{name}.codec.json"));

        cmd()
            .args(["convert", &input])
            .args(["-o", output.to_str().unwrap()])
            .args(["--codec", codec.to_str().unwrap()])
            .assert()
            .success();

        validate_outputs(name, &output, &codec);
    }

    // Warned fixtures: exit 0 with advisory diagnostics on stderr, output is valid
    for name in warned_fixtures() {
        let input = format!("{FIXTURES_DIR}/{name}.json");
        let output = dir.path().join(format!("{name}.converted.json"));
        let codec = dir.path().join(format!("{name}.codec.json"));

        cmd()
            .args(["convert", &input])
            .args(["-o", output.to_str().unwrap()])
            .args(["--codec", codec.to_str().unwrap()])
            .assert()
            .success()
            .stderr(predicate::str::contains(
                "Provider compatibility diagnostics",
            ));

        validate_outputs(name, &output, &codec);
    }
}

// ── E2E: Convert+Rehydrate roundtrip via CLI ────────────────────────────────

#[test]
fn test_cli_e2e_roundtrip() {
    let dir = TempDir::new().unwrap();
    let input = format!("{FIXTURES_DIR}/simple.json");
    let converted = dir.path().join("converted.json");
    let codec = dir.path().join("codec.json");
    let llm_output = dir.path().join("llm_output.json");
    let rehydrated = dir.path().join("rehydrated.json");

    // Step 1: Convert
    cmd()
        .args(["convert", &input])
        .args(["-o", converted.to_str().unwrap()])
        .args(["--codec", codec.to_str().unwrap()])
        .assert()
        .success();

    // Step 2: Create simulated LLM output
    let llm_data = serde_json::json!({
        "name": "Integration Test",
        "age": 42,
        "email": null,
        "active": null
    });
    fs::write(&llm_output, llm_data.to_string()).unwrap();

    // Step 3: Rehydrate (--schema is required for type coercion)
    cmd()
        .args(["rehydrate", llm_output.to_str().unwrap()])
        .args(["--codec", codec.to_str().unwrap()])
        .args(["--schema", &input])
        .args(["-o", rehydrated.to_str().unwrap()])
        .assert()
        .success();

    let content = fs::read_to_string(&rehydrated).unwrap();
    let data: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(data["name"], serde_json::json!("Integration Test"));
    assert_eq!(data["age"], serde_json::json!(42));
}

// ── E2E: Multi-target CLI ───────────────────────────────────────────────────

#[test]
fn test_cli_e2e_multi_target() {
    let dir = TempDir::new().unwrap();
    // Use simple.json for multi-target — it's clean across all providers
    let input = format!("{FIXTURES_DIR}/simple.json");

    for target in ["openai-strict", "gemini", "claude"] {
        let output = dir.path().join(format!("{target}.json"));
        cmd()
            .args(["convert", &input, "--target", target])
            .args(["-o", output.to_str().unwrap()])
            .assert()
            .success();

        let content = fs::read_to_string(&output).unwrap();
        let _: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
    }
}

// ── E2E: Error path — malformed input via CLI ───────────────────────────────

#[test]
fn test_cli_e2e_malformed_input() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("malformed.json");
    fs::write(&input, "this is not valid JSON at all {{{").unwrap();

    cmd()
        .args(["convert", input.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::is_empty().not());
}

// ── E2E: Stdout piping ─────────────────────────────────────────────────────

#[test]
fn test_cli_e2e_stdout_pipe() {
    let input = format!("{FIXTURES_DIR}/simple.json");

    cmd()
        .args(["convert", &input])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"type\""))
        .stdout(predicate::str::contains("\"additionalProperties\""));
}

// ── E2E: OAS 3.1 fixture generation (#185) ─────────────────────────────────

const OAS31_SOURCE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/oas31/source/oas31-schema.json"
);

#[test]
fn test_cli_e2e_oas31_fixtures() {
    let dir = TempDir::new().unwrap();
    let out_dir = dir.path().join("oas31");

    // Run convert --output-dir against the committed OAS 3.1 source schema
    // Note: some deeply recursive components (e.g., response-or-reference) may
    // emit "Component error" on stderr — this is expected for pathologically
    // recursive schemas. The CLI still exits 0.
    cmd()
        .args(["convert", OAS31_SOURCE])
        .args(["--output-dir", out_dir.to_str().unwrap()])
        .assert()
        .success();

    // Root files must exist
    assert!(out_dir.join("schema.json").exists(), "root schema.json");
    assert!(out_dir.join("codec.json").exists(), "root codec.json");

    // Manifest must exist and be valid
    let manifest_path = out_dir.join("manifest.json");
    assert!(manifest_path.exists(), "manifest.json must exist");

    let manifest_content = fs::read_to_string(&manifest_path).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_content).expect("manifest.json should be valid JSON");

    // Must have at least one component
    let components = manifest["components"]
        .as_array()
        .expect("components should be an array");
    assert!(
        !components.is_empty(),
        "OAS 3.1 schema should produce at least one component"
    );

    // Verify every component listed in the manifest has schema.json and codec.json
    for comp in components {
        let schema_path_str = comp["schemaPath"]
            .as_str()
            .expect("schemaPath should be a string");
        let codec_path_str = comp["codecPath"]
            .as_str()
            .expect("codecPath should be a string");

        let schema_file = out_dir.join(schema_path_str);
        let codec_file = out_dir.join(codec_path_str);

        assert!(
            schema_file.exists(),
            "Component schema missing: {}",
            schema_path_str
        );
        assert!(
            codec_file.exists(),
            "Component codec missing: {}",
            codec_path_str
        );

        // Validate both are parseable JSON
        let s_content = fs::read_to_string(&schema_file).unwrap();
        let _: serde_json::Value =
            serde_json::from_str(&s_content).expect("component schema should be valid JSON");

        let c_content = fs::read_to_string(&codec_file).unwrap();
        let _: serde_json::Value =
            serde_json::from_str(&c_content).expect("component codec should be valid JSON");
    }

    // Verify root schema and codec are valid JSON too
    let root_schema = fs::read_to_string(out_dir.join("schema.json")).unwrap();
    let _: serde_json::Value =
        serde_json::from_str(&root_schema).expect("root schema should be valid JSON");

    let root_codec = fs::read_to_string(out_dir.join("codec.json")).unwrap();
    let _: serde_json::Value =
        serde_json::from_str(&root_codec).expect("root codec should be valid JSON");
}
