pub mod java;
pub mod python;
pub mod typescript;

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Build tool for the generated SDK project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BuildTool {
    Maven,
    Setuptools,
    Npm,
}

impl std::fmt::Display for BuildTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildTool::Maven => write!(f, "maven"),
            BuildTool::Setuptools => write!(f, "setuptools"),
            BuildTool::Npm => write!(f, "npm"),
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
    }
}
