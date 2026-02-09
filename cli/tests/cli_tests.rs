//! CLI binary integration tests using assert_cmd + predicates.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("jsonschema-llm").expect("binary should exist")
}

fn simple_schema() -> String {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "integer" }
        },
        "required": ["name"]
    })
    .to_string()
}

// ── Convert to File ─────────────────────────────────────────────────────────

#[test]
fn test_convert_to_file() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    let output = dir.path().join("out.json");
    let codec = dir.path().join("codec.json");

    fs::write(&input, simple_schema()).unwrap();

    cmd()
        .args(["convert", input.to_str().unwrap()])
        .args(["-o", output.to_str().unwrap()])
        .args(["--codec", codec.to_str().unwrap()])
        .assert()
        .success();

    // Both files should exist and be valid JSON
    let out_content = fs::read_to_string(&output).expect("output file should exist");
    let _: serde_json::Value =
        serde_json::from_str(&out_content).expect("output should be valid JSON");

    let codec_content = fs::read_to_string(&codec).expect("codec file should exist");
    let _: serde_json::Value =
        serde_json::from_str(&codec_content).expect("codec should be valid JSON");
}

// ── Convert to Stdout ───────────────────────────────────────────────────────

#[test]
fn test_convert_to_stdout() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    fs::write(&input, simple_schema()).unwrap();

    cmd()
        .args(["convert", input.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"type\""));
}

// ── Rehydrate ───────────────────────────────────────────────────────────────

#[test]
fn test_rehydrate() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    let converted = dir.path().join("converted.json");
    let codec_file = dir.path().join("codec.json");
    let llm_output = dir.path().join("llm_output.json");
    let rehydrated = dir.path().join("rehydrated.json");

    // Step 1: Convert
    fs::write(&input, simple_schema()).unwrap();
    cmd()
        .args(["convert", input.to_str().unwrap()])
        .args(["-o", converted.to_str().unwrap()])
        .args(["--codec", codec_file.to_str().unwrap()])
        .assert()
        .success();

    // Step 2: Create fake LLM output matching the converted schema
    let llm_data = serde_json::json!({
        "name": "Alice",
        "age": 30
    });
    fs::write(&llm_output, llm_data.to_string()).unwrap();

    // Step 3: Rehydrate
    cmd()
        .args(["rehydrate", llm_output.to_str().unwrap()])
        .args(["--codec", codec_file.to_str().unwrap()])
        .args(["-o", rehydrated.to_str().unwrap()])
        .assert()
        .success();

    let content = fs::read_to_string(&rehydrated).expect("rehydrated file should exist");
    let data: serde_json::Value =
        serde_json::from_str(&content).expect("rehydrated output should be valid JSON");
    assert_eq!(data["name"], serde_json::json!("Alice"));
}

// ── Target Flag ─────────────────────────────────────────────────────────────

#[test]
fn test_target_flag() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    fs::write(&input, simple_schema()).unwrap();

    cmd()
        .args(["convert", input.to_str().unwrap(), "--target", "gemini"])
        .assert()
        .success();
}

// ── Invalid Input ───────────────────────────────────────────────────────────

#[test]
fn test_invalid_input() {
    cmd()
        .args(["convert", "/nonexistent/path/schema.json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to open input file"));
}

// ── Help Output ─────────────────────────────────────────────────────────────

#[test]
fn test_help_output() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("convert"))
        .stdout(predicate::str::contains("rehydrate"));
}

#[test]
fn test_convert_help() {
    cmd()
        .args(["convert", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--target"))
        .stdout(predicate::str::contains("--codec"))
        .stdout(predicate::str::contains("--polymorphism"));
}
