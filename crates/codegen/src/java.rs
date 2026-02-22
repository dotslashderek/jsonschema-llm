use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use heck::ToUpperCamelCase;
use rust_embed::Embed;
use serde::Serialize;
use tera::Tera;

use crate::{Manifest, SdkConfig};

#[derive(Embed)]
#[folder = "templates/java/"]
struct JavaTemplates;

/// Template context for generating the pom.xml.
#[derive(Serialize)]
struct PomContext {
    group_id: String,
    artifact_id: String,
    engine_version: String,
}

/// Template context for the Generator facade class.
#[derive(Serialize)]
struct GeneratorContext {
    package_name: String,
    components: Vec<ComponentContext>,
}

/// Template context for a single component class.
#[derive(Serialize)]
struct ComponentContext {
    package_name: String,
    class_name: String,
    component_name: String,
    schema_path: String,
    codec_path: String,
    original_path: String,
}

/// Generate a Java Maven SDK project.
pub fn generate(config: &SdkConfig) -> Result<()> {
    // Read and parse manifest
    let manifest_path = config.schema_dir.join("manifest.json");
    let manifest_content = fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read manifest at {}", manifest_path.display()))?;
    let manifest: Manifest =
        serde_json::from_str(&manifest_content).with_context(|| "Failed to parse manifest.json")?;

    // Build Tera engine from embedded templates
    let mut tera = Tera::default();
    for file_name in JavaTemplates::iter() {
        let file = JavaTemplates::get(&file_name)
            .with_context(|| format!("Failed to load embedded template: {}", file_name))?;
        let content = std::str::from_utf8(file.data.as_ref())
            .with_context(|| format!("Template {} is not valid UTF-8", file_name))?;
        tera.add_raw_template(&file_name, content)
            .with_context(|| format!("Failed to register template: {}", file_name))?;
    }

    // Create output directory
    fs::create_dir_all(&config.output_dir).with_context(|| {
        format!(
            "Failed to create output dir: {}",
            config.output_dir.display()
        )
    })?;

    // Generate pom.xml
    let pom_ctx = PomContext {
        group_id: config.package.clone(),
        artifact_id: config.artifact_name.clone(),
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    render_to_file(
        &tera,
        "pom.xml.tera",
        &pom_ctx,
        &config.output_dir.join("pom.xml"),
    )?;

    // Create Java source directory
    let package_dir = config.package.replace('.', "/");
    let src_dir = config.output_dir.join("src/main/java").join(&package_dir);
    fs::create_dir_all(&src_dir)
        .with_context(|| format!("Failed to create source dir: {}", src_dir.display()))?;

    // Create resources directory and copy schemas
    let resources_dir = config.output_dir.join("src/main/resources/schemas");
    fs::create_dir_all(&resources_dir).with_context(|| {
        format!(
            "Failed to create resources dir: {}",
            resources_dir.display()
        )
    })?;

    // Build component contexts and generate component classes
    let mut component_contexts = Vec::new();
    for component in &manifest.components {
        // Validate paths are relative and don't contain traversal
        for path in [&component.schema_path, &component.codec_path] {
            if path.contains("..") || path.starts_with('/') {
                anyhow::bail!(
                    "Invalid path in manifest: '{}' (must be relative, no traversal)",
                    path
                );
            }
        }

        let class_name = component.name.to_upper_camel_case();
        let ctx = ComponentContext {
            package_name: config.package.clone(),
            class_name: class_name.clone(),
            component_name: component.name.clone(),
            schema_path: component.schema_path.clone(),
            codec_path: component.codec_path.clone(),
            original_path: component.original_path.clone(),
        };

        render_to_file(
            &tera,
            "Component.java.tera",
            &ctx,
            &src_dir.join(format!("{}.java", class_name)),
        )?;

        // Copy schema, codec, and original files to resources
        copy_schema_file(&config.schema_dir, &component.schema_path, &resources_dir)?;
        copy_schema_file(&config.schema_dir, &component.codec_path, &resources_dir)?;
        copy_schema_file(&config.schema_dir, &component.original_path, &resources_dir)?;

        component_contexts.push(ctx);
    }

    // Generate the Generator facade class
    let gen_ctx = GeneratorContext {
        package_name: config.package.clone(),
        components: component_contexts,
    };
    render_to_file(
        &tera,
        "Generator.java.tera",
        &gen_ctx,
        &src_dir.join("SchemaGenerator.java"),
    )?;

    // Generate README
    let readme_ctx = tera::Context::from_serialize(&gen_ctx)?;
    let readme_content = tera.render("README.md.tera", &readme_ctx)?;
    fs::write(config.output_dir.join("README.md"), readme_content)?;

    // Generate .gitignore
    let gitignore_content = tera.render("gitignore.tera", &tera::Context::new())?;
    fs::write(config.output_dir.join(".gitignore"), gitignore_content)?;

    // Optionally git init
    if config.git_init {
        std::process::Command::new("git")
            .arg("init")
            .current_dir(&config.output_dir)
            .output()
            .context("Failed to run git init")?;
    }

    Ok(())
}

/// Render a Tera template to a file.
fn render_to_file<T: Serialize>(
    tera: &Tera,
    template_name: &str,
    context: &T,
    output_path: &Path,
) -> Result<()> {
    let ctx = tera::Context::from_serialize(context)?;
    let rendered = tera
        .render(template_name, &ctx)
        .with_context(|| format!("Failed to render template: {}", template_name))?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(output_path, rendered)
        .with_context(|| format!("Failed to write: {}", output_path.display()))?;

    Ok(())
}

/// Copy a schema/codec file from the source directory to the resources directory,
/// preserving the relative path structure.
/// Returns a hard error if the source file does not exist, ensuring broken SDK
/// packages are never silently emitted.
fn copy_schema_file(schema_dir: &Path, relative_path: &str, resources_dir: &Path) -> Result<()> {
    let src = schema_dir.join(relative_path);
    let dst = resources_dir.join(relative_path);

    if !src.exists() {
        anyhow::bail!(
            "Schema file not found: '{}' (referenced in manifest but missing from schema directory '{}')",
            src.display(),
            schema_dir.display()
        );
    }

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&src, &dst)
        .with_context(|| format!("Failed to copy {} to {}", src.display(), dst.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BuildTool;
    use tempfile::TempDir;

    #[test]
    fn generate_produces_valid_project_structure() {
        let tmp = TempDir::new().unwrap();
        let schema_dir = tmp.path().join("schemas");
        fs::create_dir_all(&schema_dir).unwrap();

        // Create a minimal manifest
        let manifest = serde_json::json!({
            "version": "1",
            "generatedAt": "2026-01-01T00:00:00Z",
            "sourceSchema": "test-schema.json",
            "target": "openai-strict",
            "mode": "strict",
            "components": [
                {
                    "name": "user-profile",
                    "pointer": "#/$defs/user-profile",
                    "schemaPath": "user-profile/schema.json",
                    "codecPath": "user-profile/codec.json",
                    "originalPath": "user-profile/original.json",
                    "dependencyCount": 0
                },
                {
                    "name": "order-item",
                    "pointer": "#/$defs/order-item",
                    "schemaPath": "order-item/schema.json",
                    "codecPath": "order-item/codec.json",
                    "originalPath": "order-item/original.json",
                    "dependencyCount": 2
                }
            ]
        });

        fs::write(
            schema_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        // Create dummy schema/codec files
        for comp in ["user-profile", "order-item"] {
            let comp_dir = schema_dir.join(comp);
            fs::create_dir_all(&comp_dir).unwrap();
            fs::write(comp_dir.join("schema.json"), "{}").unwrap();
            fs::write(comp_dir.join("codec.json"), "{}").unwrap();
            fs::write(comp_dir.join("original.json"), "{}").unwrap();
        }

        let output_dir = tmp.path().join("output");
        let config = SdkConfig {
            package: "com.example.test".to_string(),
            artifact_name: "test-sdk".to_string(),
            schema_dir,
            output_dir: output_dir.clone(),
            git_init: false,
            build_tool: BuildTool::Maven,
        };

        generate(&config).expect("generate should succeed");

        // Verify project structure
        assert!(output_dir.join("pom.xml").exists(), "pom.xml should exist");
        assert!(
            output_dir.join("README.md").exists(),
            "README.md should exist"
        );
        assert!(
            output_dir.join(".gitignore").exists(),
            ".gitignore should exist"
        );

        let java_src = output_dir.join("src/main/java/com/example/test");
        assert!(
            java_src.join("UserProfile.java").exists(),
            "UserProfile.java should exist"
        );
        assert!(
            java_src.join("OrderItem.java").exists(),
            "OrderItem.java should exist"
        );
        assert!(
            java_src.join("SchemaGenerator.java").exists(),
            "SchemaGenerator.java should exist"
        );

        // Verify pom.xml contains correct artifact info
        let pom_content = fs::read_to_string(output_dir.join("pom.xml")).unwrap();
        assert!(
            pom_content.contains("com.example.test"),
            "pom.xml should contain group ID"
        );
        assert!(
            pom_content.contains("test-sdk"),
            "pom.xml should contain artifact ID"
        );
        assert!(
            pom_content.contains("json-schema-llm-engine"),
            "pom.xml should reference engine dependency"
        );

        // Verify schema resources copied
        let resources = output_dir.join("src/main/resources/schemas");
        assert!(resources.join("user-profile/schema.json").exists());
        assert!(resources.join("order-item/codec.json").exists());
    }

    #[test]
    fn missing_schema_file_returns_error() {
        let tmp = TempDir::new().unwrap();
        let schema_dir = tmp.path().join("schemas");
        fs::create_dir_all(&schema_dir).unwrap();

        // Manifest references a component whose files do NOT exist on disk
        let manifest = serde_json::json!({
            "version": "1",
            "generatedAt": "2026-01-01T00:00:00Z",
            "sourceSchema": "test.json",
            "target": "openai-strict",
            "mode": "strict",
            "components": [
                {
                    "name": "ghost",
                    "pointer": "#/$defs/ghost",
                    "schemaPath": "ghost/schema.json",
                    "codecPath": "ghost/codec.json",
                    "originalPath": "ghost/original.json",
                    "dependencyCount": 0
                }
            ]
        });
        fs::write(
            schema_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        // Intentionally NOT creating ghost/schema.json or ghost/codec.json

        let output_dir = tmp.path().join("output");
        let config = SdkConfig {
            package: "com.example.ghost".to_string(),
            artifact_name: "ghost-sdk".to_string(),
            schema_dir,
            output_dir,
            git_init: false,
            build_tool: BuildTool::Maven,
        };

        let err =
            generate(&config).expect_err("generate should fail when schema files are missing");
        let msg = err.to_string();
        assert!(
            msg.contains("Schema file not found"),
            "error message should mention missing file, got: {msg}"
        );
    }
}
