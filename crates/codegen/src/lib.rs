pub mod java;
pub mod python;
pub mod ruby;
pub mod typescript;

use std::path::PathBuf;

use anyhow::Result;
use heck::{ToLowerCamelCase, ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
use serde::{Deserialize, Serialize};

/// Build tool for the generated SDK project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BuildTool {
    Maven,
    Setuptools,
    Npm,
    Bundler,
}

impl std::fmt::Display for BuildTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildTool::Maven => write!(f, "maven"),
            BuildTool::Setuptools => write!(f, "setuptools"),
            BuildTool::Npm => write!(f, "npm"),
            BuildTool::Bundler => write!(f, "bundler"),
        }
    }
}

/// Configuration for SDK generation.
#[derive(Debug, Clone)]
pub struct SdkConfig {
    /// Java package name, e.g. "com.example.petstore"
    pub package: String,
    /// Maven artifact name, e.g. "petstore-sdk"
    pub artifact_name: String,
    /// Directory containing manifest.json and component schemas (output of `convert --output-dir`)
    pub schema_dir: PathBuf,
    /// Output directory for the generated SDK project
    pub output_dir: PathBuf,
    /// Whether to initialize a git repository
    pub git_init: bool,
    /// Build tool to use
    pub build_tool: BuildTool,
}

/// A component entry from manifest.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestComponent {
    pub name: String,
    pub pointer: String,
    pub schema_path: String,
    pub codec_path: String,
    pub original_path: String,
    pub dependency_count: usize,
}

/// Parsed manifest.json structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub version: String,
    pub generated_at: String,
    pub source_schema: String,
    pub target: String,
    pub mode: String,
    pub components: Vec<ManifestComponent>,
}

/// Generate an SDK project from a manifest and component schemas.
pub fn generate(config: &SdkConfig) -> Result<()> {
    match config.build_tool {
        BuildTool::Maven => java::generate(config),
        BuildTool::Setuptools => python::generate(config),
        BuildTool::Npm => typescript::generate(config),
        BuildTool::Bundler => ruby::generate(config),
    }
}

/// Sanitize an arbitrary string into a valid identifier suitable for most languages.
/// - Replaces non-alphanumeric characters with `_`
/// - If the result starts with a digit, prefixes it with `_`
pub fn sanitize_identifier(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_alphanumeric() {
            if i == 0 && c.is_ascii_digit() {
                sanitized.push('_');
            }
            sanitized.push(c);
        } else {
            sanitized.push('_');
        }
    }
    // Prevent empty identifiers
    if sanitized.is_empty() {
        sanitized.push_str("_empty");
    }
    sanitized
}

/// A wrapper struct for resolving identifier collisions.
#[derive(Debug, Clone)]
pub struct ResolvedComponent {
    /// The exact, verbatim name from the manifest
    pub original_name: String,
    /// UpperCamelCase suitable for class/struct names (e.g. `UserProfile`)
    pub class_name: String,
    /// SHOUTY_SNAKE_CASE suitable for enum variants/constants (e.g. `USER_PROFILE`)
    pub enum_name: String,
    /// snake_case suitable for module/file names (e.g. `user_profile`)
    pub module_name: String,
    /// lowerCamelCase suitable for JS/TS module names (e.g. `userProfile`)
    pub module_name_camel: String,
}

/// Given a list of original component names, returns a list of resolved components
/// where any collisions within a specific casing convention are deduplicated by appending `_2`, `_3`, etc.
/// Order of the input list is preserved.
pub fn resolve_collisions<'a, I>(names: I) -> Vec<ResolvedComponent>
where
    I: IntoIterator<Item = &'a String>,
{
    // Track usage to detect collisions for each formatting style independently.
    // We use lowercased keys here specifically to prevent filesystem collisions
    // on case-insensitive OSes if any formats end up being only case-different.
    let mut seen_class: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut seen_enum: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut seen_module: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut seen_module_camel: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    let mut resolved = Vec::new();

    for name in names {
        // Step 1: Sanitize completely unsafe characters (e.g., `-` to `_`)
        let sanitized = sanitize_identifier(name);

        // Step 2: Generate the base formats
        let base_class = sanitized.to_upper_camel_case();
        let base_enum = sanitized.to_shouty_snake_case();
        let base_module = sanitized.to_snake_case();
        let base_module_camel = sanitized.to_lower_camel_case();

        // Step 3: Deduplicate Class Name
        let mut class_name = base_class.clone();
        let lower = class_name.to_ascii_lowercase();
        let counter = seen_class.entry(lower.clone()).or_insert(0);
        *counter += 1;
        if *counter > 1 {
            class_name.push_str(&format!("_{}", *counter));
            seen_class.insert(class_name.to_ascii_lowercase(), 1);
        }

        // Step 4: Deduplicate Enum Name
        let mut enum_name = base_enum.clone();
        let lower = enum_name.to_ascii_lowercase();
        let counter = seen_enum.entry(lower.clone()).or_insert(0);
        *counter += 1;
        if *counter > 1 {
            enum_name.push_str(&format!("_{}", *counter));
            seen_enum.insert(enum_name.to_ascii_lowercase(), 1);
        }

        // Step 5: Deduplicate Module Name
        let mut module_name = base_module.clone();
        let lower = module_name.to_ascii_lowercase();
        let counter = seen_module.entry(lower.clone()).or_insert(0);
        *counter += 1;
        if *counter > 1 {
            module_name.push_str(&format!("_{}", *counter));
            seen_module.insert(module_name.to_ascii_lowercase(), 1);
        }

        // Step 6: Deduplicate Module Name Camel
        let mut module_name_camel = base_module_camel.clone();
        let lower = module_name_camel.to_ascii_lowercase();
        let counter = seen_module_camel.entry(lower.clone()).or_insert(0);
        *counter += 1;
        if *counter > 1 {
            module_name_camel.push_str(&format!("_{}", *counter));
            seen_module_camel.insert(module_name_camel.to_ascii_lowercase(), 1);
        }

        resolved.push(ResolvedComponent {
            original_name: name.clone(),
            class_name,
            enum_name,
            module_name,
            module_name_camel,
        });
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_identifier() {
        assert_eq!(sanitize_identifier("user-profile"), "user_profile");
        assert_eq!(sanitize_identifier("Reference"), "Reference");
        assert_eq!(sanitize_identifier("123name"), "_123name");
        assert_eq!(sanitize_identifier("JSON Schema"), "JSON_Schema");
        assert_eq!(sanitize_identifier("!@#$"), "____");
    }

    #[test]
    fn test_resolve_collisions() {
        let names = vec![
            "Reference".to_string(),
            "reference".to_string(),
            "user-profile".to_string(),
            "REFERENCE".to_string(),
        ];

        let resolved = resolve_collisions(&names);

        // Exact name
        assert_eq!(resolved[0].original_name, "Reference");
        assert_eq!(resolved[1].original_name, "reference");

        // Class names (UpperCamelCase)
        assert_eq!(resolved[0].class_name, "Reference");
        assert_eq!(resolved[1].class_name, "Reference_2");
        assert_eq!(resolved[2].class_name, "UserProfile");
        assert_eq!(resolved[3].class_name, "Reference_3");

        // Enum names (SHOUTY_SNAKE_CASE)
        assert_eq!(resolved[0].enum_name, "REFERENCE");
        assert_eq!(resolved[1].enum_name, "REFERENCE_2");
        assert_eq!(resolved[2].enum_name, "USER_PROFILE");
        assert_eq!(resolved[3].enum_name, "REFERENCE_3");

        // Module names (snake_case)
        assert_eq!(resolved[0].module_name, "reference");
        assert_eq!(resolved[1].module_name, "reference_2");
        assert_eq!(resolved[2].module_name, "user_profile");
        assert_eq!(resolved[3].module_name, "reference_3");
    }
}
