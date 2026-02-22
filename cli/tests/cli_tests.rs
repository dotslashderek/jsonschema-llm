//! CLI binary integration tests using assert_cmd + predicates.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[allow(deprecated)]
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

    // Step 3: Rehydrate (--schema is required for type coercion)
    cmd()
        .args(["rehydrate", llm_output.to_str().unwrap()])
        .args(["--codec", codec_file.to_str().unwrap()])
        .args(["--schema", input.to_str().unwrap()])
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
        .stdout(predicate::str::contains("--polymorphism"))
        .stdout(predicate::str::contains("--output-dir"));
}

// ── #178: Extract subcommand ────────────────────────────────────────────────

fn schema_with_defs() -> String {
    serde_json::json!({
        "type": "object",
        "properties": {
            "pet": { "$ref": "#/$defs/Pet" }
        },
        "$defs": {
            "Pet": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "tag": { "$ref": "#/$defs/Tag" }
                },
                "required": ["name"]
            },
            "Tag": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "label": { "type": "string" }
                }
            }
        }
    })
    .to_string()
}

#[test]
fn test_extract_to_stdout() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    fs::write(&input, schema_with_defs()).unwrap();

    cmd()
        .args(["extract", input.to_str().unwrap()])
        .args(["--pointer", "#/$defs/Pet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"type\""))
        .stdout(predicate::str::contains("\"name\""));
}

#[test]
fn test_extract_to_file() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    let output = dir.path().join("pet.json");
    fs::write(&input, schema_with_defs()).unwrap();

    cmd()
        .args(["extract", input.to_str().unwrap()])
        .args(["--pointer", "#/$defs/Pet"])
        .args(["-o", output.to_str().unwrap()])
        .assert()
        .success();

    let content = fs::read_to_string(&output).expect("output file should exist");
    let val: serde_json::Value =
        serde_json::from_str(&content).expect("output should be valid JSON");
    // Extracted Pet should have Tag in $defs
    assert!(val["$defs"]["Tag"].is_object(), "Tag dep should be inlined");
}

#[test]
fn test_extract_missing_pointer() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    fs::write(&input, schema_with_defs()).unwrap();

    cmd()
        .args(["extract", input.to_str().unwrap()])
        .args(["--pointer", "#/$defs/DoesNotExist"])
        .assert()
        .failure()
        .stderr(predicate::str::is_empty().not());
}

// ── #178: ListComponents subcommand ─────────────────────────────────────────

#[test]
fn test_list_components() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    fs::write(&input, schema_with_defs()).unwrap();

    let output = cmd()
        .args(["list-components", input.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();

    assert_eq!(lines.len(), 2, "should list 2 components");
    // Sorted: Pet < Tag
    assert_eq!(lines[0], "#/$defs/Pet");
    assert_eq!(lines[1], "#/$defs/Tag");
}

#[test]
fn test_list_components_empty() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    fs::write(&input, simple_schema()).unwrap();

    let output = cmd()
        .args(["list-components", input.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(stdout.trim().is_empty(), "no defs → empty output");
}

// ── #178: Convert --output-dir ──────────────────────────────────────────────

#[test]
fn test_convert_output_dir() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    let out_dir = dir.path().join("output");
    fs::write(&input, schema_with_defs()).unwrap();

    cmd()
        .args(["convert", input.to_str().unwrap()])
        .args(["--output-dir", out_dir.to_str().unwrap()])
        .assert()
        .success();

    // Root files
    assert!(out_dir.join("schema.json").exists(), "root schema.json");
    assert!(out_dir.join("codec.json").exists(), "root codec.json");
    // Component directories (full pointer path: $defs/Pet, $defs/Tag)
    assert!(
        out_dir.join("$defs/Pet/schema.json").exists(),
        "Pet schema.json"
    );
    assert!(
        out_dir.join("$defs/Pet/codec.json").exists(),
        "Pet codec.json"
    );
    assert!(
        out_dir.join("$defs/Tag/schema.json").exists(),
        "Tag schema.json"
    );
}

#[test]
fn test_convert_output_dir_skip_components() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    let out_dir = dir.path().join("output");
    fs::write(&input, schema_with_defs()).unwrap();

    cmd()
        .args(["convert", input.to_str().unwrap()])
        .args(["--output-dir", out_dir.to_str().unwrap()])
        .args(["--skip-components"])
        .assert()
        .success();

    // Root files should exist
    assert!(out_dir.join("schema.json").exists());
    assert!(out_dir.join("codec.json").exists());
    // No component directories
    assert!(
        !out_dir.join("$defs").exists(),
        "skip-components should suppress component dirs"
    );
}

// ── #179: Manifest ──────────────────────────────────────────────────────────

#[test]
fn test_convert_output_dir_manifest() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    let out_dir = dir.path().join("output");
    fs::write(&input, schema_with_defs()).unwrap();

    cmd()
        .args(["convert", input.to_str().unwrap()])
        .args(["--output-dir", out_dir.to_str().unwrap()])
        .assert()
        .success();

    let manifest_path = out_dir.join("manifest.json");
    assert!(manifest_path.exists(), "manifest.json should exist");

    let content = fs::read_to_string(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(manifest["version"], "1");
    assert!(manifest["generatedAt"].is_string());
    assert_eq!(manifest["target"], "openai-strict");
    assert_eq!(manifest["mode"], "strict");

    let components = manifest["components"].as_array().unwrap();
    assert_eq!(components.len(), 2, "should list Pet and Tag");

    // Verify component entries have expected fields
    for comp in components {
        assert!(comp["name"].is_string());
        assert!(comp["pointer"].is_string());
        assert!(comp["schemaPath"].is_string());
        assert!(comp["codecPath"].is_string());
        assert!(comp["dependencyCount"].is_number());
    }
}

// ── Mutual exclusion: --output-dir vs -o ────────────────────────────────────

#[test]
fn test_output_dir_conflicts_with_output() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    fs::write(&input, simple_schema()).unwrap();

    cmd()
        .args(["convert", input.to_str().unwrap()])
        .args(["--output-dir", dir.path().join("out").to_str().unwrap()])
        .args(["-o", dir.path().join("file.json").to_str().unwrap()])
        .assert()
        .failure();
}

// ── Collision safety: same name at different paths ──────────────────────────

#[test]
fn test_output_dir_collision_safety() {
    let schema = serde_json::json!({
        "type": "object",
        "$defs": {
            "User": { "type": "object", "properties": { "name": { "type": "string" } } }
        },
        "components": {
            "schemas": {
                "User": { "type": "object", "properties": { "email": { "type": "string" } } }
            }
        }
    })
    .to_string();

    let dir = TempDir::new().unwrap();
    let input = dir.path().join("schema.json");
    let out_dir = dir.path().join("output");
    fs::write(&input, &schema).unwrap();

    cmd()
        .args(["convert", input.to_str().unwrap()])
        .args(["--output-dir", out_dir.to_str().unwrap()])
        .assert()
        .success();

    // Both should exist without collision
    assert!(out_dir.join("$defs/User/schema.json").exists());
    assert!(out_dir.join("components/schemas/User/schema.json").exists());
}

// ── Help shows new subcommands ──────────────────────────────────────────────

#[test]
fn test_help_shows_new_subcommands() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("extract"))
        .stdout(predicate::str::contains("list-components"));
}

// ── #186: gen-sdk Python support ────────────────────────────────────────────

fn setup_gen_sdk_fixtures(dir: &TempDir) -> std::path::PathBuf {
    let schema_dir = dir.path().join("converted");
    fs::create_dir_all(&schema_dir).unwrap();

    let manifest = serde_json::json!({
        "version": "1",
        "generatedAt": "2026-01-01T00:00:00Z",
        "sourceSchema": "test.json",
        "target": "openai-strict",
        "mode": "strict",
        "components": [
            {
                "name": "user-profile",
                "pointer": "#/$defs/user-profile",
                "schemaPath": "user-profile/schema.json",
                "codecPath": "user-profile/codec.json",
                "dependencyCount": 0
            }
        ]
    });
    fs::write(
        schema_dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let comp_dir = schema_dir.join("user-profile");
    fs::create_dir_all(&comp_dir).unwrap();
    fs::write(comp_dir.join("schema.json"), "{}").unwrap();
    fs::write(comp_dir.join("codec.json"), "{}").unwrap();

    schema_dir
}

#[test]
fn test_gen_sdk_python_produces_valid_project() {
    let dir = TempDir::new().unwrap();
    let schema_dir = setup_gen_sdk_fixtures(&dir);
    let output = dir.path().join("my-sdk");

    cmd()
        .args(["gen-sdk", "--language", "python"])
        .args(["--schema", schema_dir.to_str().unwrap()])
        .args(["--package", "my-test-sdk"])
        .args(["--output", output.to_str().unwrap()])
        .args(["--build-tool", "setuptools"])
        .assert()
        .success()
        .stderr(predicate::str::contains("SDK generated successfully"));

    assert!(output.join("pyproject.toml").exists());
    assert!(output.join("README.md").exists());
    assert!(output.join(".gitignore").exists());
    // snake_case import name directory
    assert!(output.join("my_test_sdk/__init__.py").exists());
    assert!(output.join("my_test_sdk/user_profile.py").exists());
    assert!(output.join("my_test_sdk/generator.py").exists());
    // Schema resources inside package
    assert!(output
        .join("my_test_sdk/schemas/user-profile/schema.json")
        .exists());
}

#[test]
fn test_gen_sdk_python_rejects_invalid_package() {
    let dir = TempDir::new().unwrap();
    let schema_dir = setup_gen_sdk_fixtures(&dir);

    // Spaces + special chars
    cmd()
        .args(["gen-sdk", "--language", "python"])
        .args(["--schema", schema_dir.to_str().unwrap()])
        .args(["--package", "my sdk!"])
        .args(["--output", dir.path().join("out").to_str().unwrap()])
        .args(["--build-tool", "setuptools"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid Python package name"));

    // PEP 508: leading dot disallowed
    cmd()
        .args(["gen-sdk", "--language", "python"])
        .args(["--schema", schema_dir.to_str().unwrap()])
        .args(["--package", ".my-sdk"])
        .args(["--output", dir.path().join("out2").to_str().unwrap()])
        .args(["--build-tool", "setuptools"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("PEP 508"));

    // PEP 508: must start with alphanumeric
    cmd()
        .args(["gen-sdk", "--language", "python"])
        .args(["--schema", schema_dir.to_str().unwrap()])
        .args(["--package", "_my_sdk"])
        .args(["--output", dir.path().join("out3").to_str().unwrap()])
        .args(["--build-tool", "setuptools"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("PEP 508"));
}

#[test]
fn test_gen_sdk_help_shows_python() {
    cmd()
        .args(["gen-sdk", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("python"))
        .stdout(predicate::str::contains("setuptools"));
}

// ── Language / build-tool coupling ─────────────────────────────────────

#[test]
fn test_gen_sdk_python_maven_combo_rejected() {
    let dir = TempDir::new().unwrap();
    let schema_dir = setup_gen_sdk_fixtures(&dir);

    cmd()
        .args(["gen-sdk", "--language", "python"])
        .args(["--schema", schema_dir.to_str().unwrap()])
        .args(["--package", "my-sdk"])
        .args(["--output", dir.path().join("out").to_str().unwrap()])
        .args(["--build-tool", "maven"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid combination"));
}

#[test]
fn test_gen_sdk_java_setuptools_combo_rejected() {
    let dir = TempDir::new().unwrap();
    let schema_dir = setup_gen_sdk_fixtures(&dir);

    cmd()
        .args(["gen-sdk", "--language", "java"])
        .args(["--schema", schema_dir.to_str().unwrap()])
        .args(["--package", "com.example.sdk"])
        .args(["--output", dir.path().join("out").to_str().unwrap()])
        .args(["--build-tool", "setuptools"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid combination"));
}

#[test]
fn test_gen_sdk_python_default_build_tool_is_setuptools() {
    let dir = TempDir::new().unwrap();
    let schema_dir = setup_gen_sdk_fixtures(&dir);
    let output = dir.path().join("out-default");

    // --build-tool omitted: should default to setuptools for python
    cmd()
        .args(["gen-sdk", "--language", "python"])
        .args(["--schema", schema_dir.to_str().unwrap()])
        .args(["--package", "my-default-sdk"])
        .args(["--output", output.to_str().unwrap()])
        .assert()
        .success();

    // pyproject.toml (not pom.xml) is the artifact of setuptools
    assert!(
        output.join("pyproject.toml").exists(),
        "should produce pyproject.toml"
    );
    assert!(
        !output.join("pom.xml").exists(),
        "should NOT produce pom.xml"
    );
}
