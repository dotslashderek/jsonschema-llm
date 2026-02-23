//! Ruby SDK generator.
//!
//! Generates a Bundler-based Ruby gem project from converted schemas.
//! Follows the same architecture as `java.rs`, `python.rs`, and `typescript.rs`.

use anyhow::{Context, Result};
use rust_embed::Embed;
use serde::Serialize;
use std::fs;
use std::path::Path;
use tera::Tera;

use crate::{Manifest, SdkConfig};

// ---------------------------------------------------------------------------
// Embedded templates
// ---------------------------------------------------------------------------

#[derive(Embed)]
#[folder = "templates/ruby/"]
struct RubyTemplates;

// ---------------------------------------------------------------------------
// Template contexts
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct GemspecContext {
    sdk_name: String,
}

#[derive(Serialize)]
struct GeneratorContext {
    sdk_name: String,
    generator_module: String,
    components: Vec<GeneratorComponent>,
}

#[derive(Serialize)]
struct GeneratorComponent {
    name: String,
    module_name: String,
    file_name: String,
}

#[derive(Serialize)]
struct ComponentContext {
    generator_module: String,
    component_name: String,
    module_name: String,
    schema_path: String,
    codec_path: String,
    original_path: String,
}

#[derive(Serialize)]
struct ReadmeComponent {
    name: String,
    module_name: String,
    description: String,
}

#[derive(Serialize)]
struct ReadmeContext {
    sdk_name: String,
    module_name: String,
    generator_module: String,
    components: Vec<ReadmeComponent>,
}

// ---------------------------------------------------------------------------
// Name helpers
// ---------------------------------------------------------------------------

/// Convert a gem name like "my-petstore-sdk" to a Ruby module name like "MyPetstoreSdk"
fn to_module_name(name: &str) -> String {
    name.split(&['-', '_'][..])
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().to_string() + &chars.collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

/// Convert a component name to a snake_case file name.
fn component_to_file_name(name: &str) -> String {
    // Convert CamelCase or PascalCase to snake_case
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(c.to_ascii_lowercase());
    }
    // Also handle hyphens → underscores
    result.replace('-', "_")
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

pub fn generate(config: &SdkConfig) -> Result<()> {
    let output_dir = &config.output_dir;
    let sdk_name = &config.package;
    let generator_module = to_module_name(sdk_name);

    // Read the manifest
    let manifest_path = config.schema_dir.join("manifest.json");
    let manifest_content = fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read manifest: {}", manifest_path.display()))?;
    let manifest: Manifest =
        serde_json::from_str(&manifest_content).context("Failed to parse manifest.json")?;

    // Create output directory structure
    let lib_dir = output_dir.join("lib");
    let module_dir = lib_dir.join(component_to_file_name(sdk_name));
    let schemas_dir = module_dir.join("schemas");
    fs::create_dir_all(&schemas_dir)
        .with_context(|| format!("Failed to create schemas dir: {}", schemas_dir.display()))?;

    // Initialize Tera
    let mut tera = Tera::default();
    for name in &[
        "gemspec.tera",
        "component.rb.tera",
        "generator.rb.tera",
        "README.md.tera",
        "gitignore.tera",
    ] {
        let content = RubyTemplates::get(name)
            .with_context(|| format!("Missing embedded template: {name}"))?;
        let content_str = std::str::from_utf8(content.data.as_ref())
            .with_context(|| format!("Invalid UTF-8 in template: {name}"))?;
        tera.add_raw_template(name, content_str)
            .with_context(|| format!("Failed to add template: {name}"))?;
    }

    // Build component data
    let mut gen_components = Vec::new();
    let mut readme_components = Vec::new();

    for comp in &manifest.components {
        let module_name = to_module_name(&comp.name);
        let file_name = component_to_file_name(&comp.name);

        gen_components.push(GeneratorComponent {
            name: comp.name.clone(),
            module_name: module_name.clone(),
            file_name: file_name.clone(),
        });

        readme_components.push(ReadmeComponent {
            name: comp.name.clone(),
            module_name: module_name.clone(),
            description: format!("Generate a {}", comp.name),
        });

        // Copy schema files for this component
        let comp_schema_dir = schemas_dir.join(&comp.name);
        fs::create_dir_all(&comp_schema_dir).with_context(|| {
            format!(
                "Failed to create component schema dir: {}",
                comp_schema_dir.display()
            )
        })?;

        copy_schema_file(
            &config.schema_dir,
            &comp_schema_dir,
            &comp.schema_path,
            "schema.json",
        )?;
        copy_schema_file(
            &config.schema_dir,
            &comp_schema_dir,
            &comp.codec_path,
            "codec.json",
        )?;
        copy_schema_file(
            &config.schema_dir,
            &comp_schema_dir,
            &comp.original_path,
            "original.json",
        )?;

        // Render component module
        let comp_ctx = ComponentContext {
            generator_module: generator_module.clone(),
            component_name: comp.name.clone(),
            module_name: module_name.clone(),
            schema_path: format!("{}/schema.json", comp.name),
            codec_path: format!("{}/codec.json", comp.name),
            original_path: format!("{}/original.json", comp.name),
        };
        render_to_file(
            &tera,
            "component.rb.tera",
            &comp_ctx,
            &module_dir.join(format!("{}.rb", file_name)),
        )?;
    }

    // Render generator facade
    let gen_ctx = GeneratorContext {
        sdk_name: sdk_name.clone(),
        generator_module: generator_module.clone(),
        components: gen_components,
    };
    render_to_file(
        &tera,
        "generator.rb.tera",
        &gen_ctx,
        &module_dir.join("generator.rb"),
    )?;

    // Render barrel require file
    let barrel_content = format!(
        "# frozen_string_literal: true\n\nrequire_relative \"{}/generator\"\n",
        component_to_file_name(sdk_name)
    );
    fs::write(
        lib_dir.join(format!("{}.rb", component_to_file_name(sdk_name))),
        &barrel_content,
    )
    .context("Failed to write barrel require file")?;

    // Render gemspec
    let gemspec_ctx = GemspecContext {
        sdk_name: sdk_name.clone(),
    };
    render_to_file(
        &tera,
        "gemspec.tera",
        &gemspec_ctx,
        &output_dir.join(format!("{}.gemspec", sdk_name)),
    )?;

    // Render README
    let readme_ctx = ReadmeContext {
        sdk_name: sdk_name.clone(),
        module_name: component_to_file_name(sdk_name),
        generator_module: generator_module.clone(),
        components: readme_components,
    };
    render_to_file(
        &tera,
        "README.md.tera",
        &readme_ctx,
        &output_dir.join("README.md"),
    )?;

    // Render .gitignore
    render_to_file(
        &tera,
        "gitignore.tera",
        &std::collections::HashMap::<String, String>::new(),
        &output_dir.join(".gitignore"),
    )?;

    // Render Gemfile
    let gemfile_content =
        "# frozen_string_literal: true\n\nsource \"https://rubygems.org\"\n\ngemspec\n";
    fs::write(output_dir.join("Gemfile"), gemfile_content).context("Failed to write Gemfile")?;

    // Initialize git if requested
    if config.git_init {
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(output_dir)
            .output()
            .context("Failed to initialize git repository")?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn render_to_file<T: Serialize>(
    tera: &Tera,
    template: &str,
    context: &T,
    output: &Path,
) -> Result<()> {
    let ctx = tera::Context::from_serialize(context)?;
    let rendered = tera.render(template, &ctx)?;
    fs::write(output, rendered)
        .with_context(|| format!("Failed to write: {}", output.display()))?;
    Ok(())
}

fn copy_schema_file(
    schema_dir: &Path,
    target_dir: &Path,
    schema_path: &str,
    target_name: &str,
) -> Result<()> {
    let source = schema_dir.join(schema_path);
    let target = target_dir.join(target_name);

    if !source.exists() {
        anyhow::bail!(
            "Schema file not found: {} (expected at {})",
            schema_path,
            source.display()
        );
    }

    fs::copy(&source, &target)
        .with_context(|| format!("Failed to copy {} → {}", source.display(), target.display()))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildTool, SdkConfig};
    use std::path::PathBuf;

    fn create_test_schema_dir(tmp: &Path) -> PathBuf {
        let schema_dir = tmp.join("schemas");
        fs::create_dir_all(schema_dir.join("$defs/Pet")).unwrap();

        // Create manifest
        let manifest = serde_json::json!({
            "version": "1",
            "generatedAt": "2024-01-01T00:00:00Z",
            "sourceSchema": "petstore.json",
            "target": "openai_strict",
            "mode": "strict",
            "components": [{
                "name": "Pet",
                "pointer": "#/$defs/Pet",
                "schemaPath": "$defs/Pet/schema.json",
                "codecPath": "$defs/Pet/codec.json",
                "originalPath": "$defs/Pet/original.json",
                "dependencyCount": 0
            }]
        });
        fs::write(
            schema_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        // Create component files
        let schema_json = serde_json::json!({"type": "object"});
        let codec_json = serde_json::json!({});
        let original_json =
            serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}}});

        fs::write(
            schema_dir.join("$defs/Pet/schema.json"),
            serde_json::to_string(&schema_json).unwrap(),
        )
        .unwrap();
        fs::write(
            schema_dir.join("$defs/Pet/codec.json"),
            serde_json::to_string(&codec_json).unwrap(),
        )
        .unwrap();
        fs::write(
            schema_dir.join("$defs/Pet/original.json"),
            serde_json::to_string(&original_json).unwrap(),
        )
        .unwrap();

        schema_dir
    }

    #[test]
    fn generate_produces_valid_project_structure() {
        let tmp = tempfile::tempdir().unwrap();
        let schema_dir = create_test_schema_dir(tmp.path());
        let output_dir = tmp.path().join("output");

        let config = SdkConfig {
            package: "my-petstore-sdk".to_string(),
            artifact_name: "my-petstore-sdk".to_string(),
            schema_dir,
            output_dir: output_dir.clone(),
            git_init: false,
            build_tool: BuildTool::Bundler,
        };

        generate(&config).unwrap();

        // Verify project structure
        assert!(output_dir.join("my-petstore-sdk.gemspec").exists());
        assert!(output_dir.join("Gemfile").exists());
        assert!(output_dir.join("README.md").exists());
        assert!(output_dir.join(".gitignore").exists());
        assert!(output_dir.join("lib/my_petstore_sdk.rb").exists());
        assert!(output_dir.join("lib/my_petstore_sdk/generator.rb").exists());
        assert!(output_dir.join("lib/my_petstore_sdk/pet.rb").exists());
        assert!(output_dir
            .join("lib/my_petstore_sdk/schemas/Pet/schema.json")
            .exists());
        assert!(output_dir
            .join("lib/my_petstore_sdk/schemas/Pet/codec.json")
            .exists());
        assert!(output_dir
            .join("lib/my_petstore_sdk/schemas/Pet/original.json")
            .exists());

        // Verify generated file content
        let component_rb =
            fs::read_to_string(output_dir.join("lib/my_petstore_sdk/pet.rb")).unwrap();
        assert!(component_rb.contains("module MyPetstoreSdk"));
        assert!(component_rb.contains("module Pet"));
        assert!(component_rb.contains("def self.schema"));
        assert!(component_rb.contains("def self.generate"));

        let generator_rb =
            fs::read_to_string(output_dir.join("lib/my_petstore_sdk/generator.rb")).unwrap();
        assert!(generator_rb.contains("module MyPetstoreSdk"));
        assert!(generator_rb.contains("COMPONENTS"));
    }

    #[test]
    fn missing_schema_file_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let schema_dir = tmp.path().join("schemas");
        fs::create_dir_all(&schema_dir).unwrap();

        // Create manifest referencing a missing file
        let manifest = serde_json::json!({
            "version": "1",
            "generatedAt": "2024-01-01T00:00:00Z",
            "sourceSchema": "test.json",
            "target": "openai_strict",
            "mode": "strict",
            "components": [{
                "name": "Missing",
                "pointer": "#/$defs/Missing",
                "schemaPath": "$defs/Missing/schema.json",
                "codecPath": "$defs/Missing/codec.json",
                "originalPath": "$defs/Missing/original.json",
                "dependencyCount": 0
            }]
        });
        fs::write(
            schema_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let config = SdkConfig {
            package: "test-sdk".to_string(),
            artifact_name: "test-sdk".to_string(),
            schema_dir,
            output_dir: tmp.path().join("output"),
            git_init: false,
            build_tool: BuildTool::Bundler,
        };

        let result = generate(&config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Schema file not found"));
    }

    #[test]
    fn to_module_name_converts_correctly() {
        assert_eq!(to_module_name("my-petstore-sdk"), "MyPetstoreSdk");
        assert_eq!(to_module_name("simple"), "Simple");
        assert_eq!(to_module_name("my_api"), "MyApi");
    }

    #[test]
    fn component_to_file_name_converts_correctly() {
        assert_eq!(component_to_file_name("Pet"), "pet");
        assert_eq!(component_to_file_name("UserProfile"), "user_profile");
        assert_eq!(component_to_file_name("my-component"), "my_component");
    }
}
