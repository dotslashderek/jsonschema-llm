use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use heck::ToLowerCamelCase;
use rust_embed::Embed;
use serde::Serialize;
use tera::Tera;

use crate::{Manifest, SdkConfig};

#[derive(Embed)]
#[folder = "templates/typescript/"]
struct TypeScriptTemplates;

/// Template context for generating package.json.
#[derive(Serialize)]
struct PackageContext {
    package_name: String,
}

/// Template context for a single component module.
#[derive(Serialize)]
struct ComponentContext {
    component_name: String,
    module_name: String,
    schema_path: String,
    codec_path: String,
}

/// Template context for the index barrel export.
#[derive(Serialize)]
struct IndexContext {
    package_name: String,
    source_schema: String,
    components: Vec<ComponentContext>,
}

/// Generate a TypeScript/Node.js SDK project.
pub fn generate(config: &SdkConfig) -> Result<()> {
    // Read and parse manifest
    let manifest_path = config.schema_dir.join("manifest.json");
    let manifest_content = fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read manifest at {}", manifest_path.display()))?;
    let manifest: Manifest =
        serde_json::from_str(&manifest_content).with_context(|| "Failed to parse manifest.json")?;

    // Build Tera engine from embedded templates
    let mut tera = Tera::default();
    for file_name in TypeScriptTemplates::iter() {
        let file = TypeScriptTemplates::get(&file_name)
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

    // Generate package.json
    let pkg_ctx = PackageContext {
        package_name: config.package.clone(),
    };
    render_to_file(
        &tera,
        "package.json.tera",
        &pkg_ctx,
        &config.output_dir.join("package.json"),
    )?;

    // Generate tsconfig.json
    render_to_file(
        &tera,
        "tsconfig.json.tera",
        &pkg_ctx,
        &config.output_dir.join("tsconfig.json"),
    )?;

    // Create src directory
    let src_dir = config.output_dir.join("src");
    fs::create_dir_all(&src_dir)
        .with_context(|| format!("Failed to create src dir: {}", src_dir.display()))?;

    // Create schemas directory
    let schemas_dir = config.output_dir.join("schemas");
    fs::create_dir_all(&schemas_dir)
        .with_context(|| format!("Failed to create schemas dir: {}", schemas_dir.display()))?;

    // Build component contexts
    let mut component_contexts: Vec<ComponentContext> = Vec::new();

    for component in &manifest.components {
        let module_name = component.name.to_lower_camel_case();

        // Validate source schema/codec files exist
        let schema_src = config.schema_dir.join(&component.schema_path);
        let codec_src = config.schema_dir.join(&component.codec_path);
        if !schema_src.exists() {
            anyhow::bail!(
                "Schema file not found for component '{}': {}",
                component.name,
                schema_src.display()
            );
        }
        if !codec_src.exists() {
            anyhow::bail!(
                "Codec file not found for component '{}': {}",
                component.name,
                codec_src.display()
            );
        }

        // Copy schema and codec to output schemas directory, preserving relative path
        let schema_dest = schemas_dir.join(&component.schema_path);
        let codec_dest = schemas_dir.join(&component.codec_path);
        if let Some(parent) = schema_dest.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(parent) = codec_dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&schema_src, &schema_dest).with_context(|| {
            format!(
                "Failed to copy schema: {} -> {}",
                schema_src.display(),
                schema_dest.display()
            )
        })?;
        fs::copy(&codec_src, &codec_dest).with_context(|| {
            format!(
                "Failed to copy codec: {} -> {}",
                codec_src.display(),
                codec_dest.display()
            )
        })?;

        let ctx = ComponentContext {
            component_name: component.name.clone(),
            module_name: module_name.clone(),
            schema_path: component.schema_path.clone(),
            codec_path: component.codec_path.clone(),
        };

        // Generate component module
        render_to_file(
            &tera,
            "component.ts.tera",
            &ctx,
            &src_dir.join(format!("{}.ts", module_name)),
        )?;

        component_contexts.push(ctx);
    }

    // Generate index.ts barrel export
    let index_ctx = IndexContext {
        package_name: config.package.clone(),
        source_schema: manifest.source_schema.clone(),
        components: component_contexts,
    };
    render_to_file(
        &tera,
        "index.ts.tera",
        &index_ctx,
        &src_dir.join("index.ts"),
    )?;

    // Generate README.md
    render_to_file(
        &tera,
        "README.md.tera",
        &index_ctx,
        &config.output_dir.join("README.md"),
    )?;

    // Generate .gitignore
    fs::write(
        config.output_dir.join(".gitignore"),
        "node_modules/\ndist/\n*.tsbuildinfo\n",
    )?;

    // Optionally initialize git
    if config.git_init {
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&config.output_dir)
            .status()
            .ok();
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
    let ctx =
        tera::Context::from_serialize(context).with_context(|| "Failed to serialize context")?;
    let rendered = tera
        .render(template_name, &ctx)
        .with_context(|| format!("Failed to render template: {}", template_name))?;
    fs::write(output_path, rendered)
        .with_context(|| format!("Failed to write: {}", output_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BuildTool;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_manifest(dir: &Path) {
        let manifest = serde_json::json!({
            "version": "1.0.0",
            "generatedAt": "2026-01-01T00:00:00Z",
            "sourceSchema": "test-schema.json",
            "target": "openai-strict",
            "mode": "strict",
            "components": [
                {
                    "name": "user-profile",
                    "pointer": "#/$defs/UserProfile",
                    "schemaPath": "$defs/UserProfile/schema.json",
                    "codecPath": "$defs/UserProfile/codec.json",
                    "dependencyCount": 0
                }
            ]
        });
        fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();

        // Create component schema/codec files
        let comp_dir = dir.join("$defs").join("UserProfile");
        fs::create_dir_all(&comp_dir).unwrap();
        fs::write(
            comp_dir.join("schema.json"),
            r#"{"type":"object","properties":{"name":{"type":"string"}}}"#,
        )
        .unwrap();
        fs::write(comp_dir.join("codec.json"), r#"{"transforms":[]}"#).unwrap();
    }

    #[test]
    fn test_generate_creates_project_structure() {
        let schema_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();
        create_test_manifest(schema_dir.path());

        let config = SdkConfig {
            package: "@my-org/test-sdk".to_string(),
            artifact_name: "test-sdk".to_string(),
            schema_dir: PathBuf::from(schema_dir.path()),
            output_dir: PathBuf::from(output_dir.path()),
            git_init: false,
            build_tool: BuildTool::Npm,
        };

        generate(&config).expect("generation should succeed");

        // Verify structure
        assert!(output_dir.path().join("package.json").exists());
        assert!(output_dir.path().join("tsconfig.json").exists());
        assert!(output_dir.path().join("src/index.ts").exists());
        assert!(output_dir.path().join("src/userProfile.ts").exists());
        assert!(output_dir
            .path()
            .join("schemas/$defs/UserProfile/schema.json")
            .exists());
        assert!(output_dir
            .path()
            .join("schemas/$defs/UserProfile/codec.json")
            .exists());
        assert!(output_dir.path().join("README.md").exists());
        assert!(output_dir.path().join(".gitignore").exists());

        // Verify package.json content
        let pkg: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(output_dir.path().join("package.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(pkg["name"], "@my-org/test-sdk");
    }

    #[test]
    fn test_generate_fails_on_missing_schema() {
        let schema_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();

        // Create manifest pointing to nonexistent files
        let manifest = serde_json::json!({
            "version": "1.0.0",
            "generatedAt": "2026-01-01T00:00:00Z",
            "sourceSchema": "test.json",
            "target": "openai-strict",
            "mode": "strict",
            "components": [
                {
                    "name": "missing",
                    "pointer": "#/$defs/Missing",
                    "schemaPath": "missing/schema.json",
                    "codecPath": "missing/codec.json",
                    "dependencyCount": 0
                }
            ]
        });
        fs::write(
            schema_dir.path().join("manifest.json"),
            manifest.to_string(),
        )
        .unwrap();

        let config = SdkConfig {
            package: "test-sdk".to_string(),
            artifact_name: "test-sdk".to_string(),
            schema_dir: PathBuf::from(schema_dir.path()),
            output_dir: PathBuf::from(output_dir.path()),
            git_init: false,
            build_tool: BuildTool::Npm,
        };

        let result = generate(&config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Schema file not found"));
    }
}
