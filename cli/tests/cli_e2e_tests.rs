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
    vec!["simple", "discriminator", "opaque", "allof", "maps"]
}

/// Fixtures that trigger advisory provider compat diagnostics (e.g. depth budget)
/// but still produce valid output. The CLI exits 0 (transforms were applied).
fn warned_fixtures() -> Vec<&'static str> {
    vec!["kitchen_sink", "recursive"]
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

    // Step 3: Rehydrate
    cmd()
        .args(["rehydrate", llm_output.to_str().unwrap()])
        .args(["--codec", codec.to_str().unwrap()])
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
