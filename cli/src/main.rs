use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use jsonschema_llm_core::config::PolymorphismStrategy;
use jsonschema_llm_core::{
    convert, convert_all_components, extract_component, list_components, rehydrate, Codec,
    ConvertOptions, ExtractOptions, Mode, Target,
};
use serde::Deserialize;
use serde_json::Value;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use tracing::level_filters::LevelFilter;

#[derive(Parser)]
#[command(name = "jsonschema-llm")]
#[command(about = "Convert any JSON Schema into an LLM-compatible structured output schema")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging (sets log level to debug)
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Convert a JSON Schema to an LLM-compatible schema
    Convert {
        /// Input JSON Schema file
        input: PathBuf,

        /// Output converted schema file (defaults to stdout if not specified)
        #[arg(short, long, conflicts_with = "output_dir")]
        output: Option<PathBuf>,

        /// Output directory for multi-file output (schema + codec + per-component)
        #[arg(long)]
        output_dir: Option<PathBuf>,

        /// Output codec (rehydration metadata) file
        #[arg(long)]
        codec: Option<PathBuf>,

        /// Target LLM provider
        #[arg(short, long, value_enum, default_value_t = TargetArg::OpenaiStrict)]
        target: TargetArg,

        /// Conversion mode (strict vs permissive)
        #[arg(long, value_enum, default_value_t = ModeArg::Strict)]
        mode: ModeArg,

        /// Polymorphism strategy
        #[arg(long, value_enum, default_value_t = PolymorphismArg::AnyOf)]
        polymorphism: PolymorphismArg,

        /// Max traversal depth for ref resolution
        #[arg(long, default_value_t = 50)]
        max_depth: usize,

        /// Recursion limit (cycles before breaking with placeholder)
        #[arg(long, default_value_t = 3)]
        recursion_limit: usize,

        /// Skip processing $defs/components entirely
        #[arg(long, default_value_t = false)]
        skip_components: bool,

        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Pretty)]
        format: OutputFormat,
    },

    /// Rehydrate LLM output back to the original schema shape
    Rehydrate {
        /// LLM output JSON file
        input: PathBuf,

        /// Codec file from conversion
        #[arg(long)]
        codec: PathBuf,

        /// Original schema file (required for type coercion during rehydration)
        #[arg(long)]
        schema: PathBuf,

        /// Output rehydrated JSON file (defaults to stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Pretty)]
        format: OutputFormat,
    },

    /// Extract a single component from a schema by JSON Pointer
    Extract {
        /// Input JSON Schema file
        input: PathBuf,

        /// JSON Pointer to the component (e.g., "#/$defs/Pet")
        #[arg(short, long)]
        pointer: String,

        /// Output file (defaults to stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Pretty)]
        format: OutputFormat,
    },

    /// List all extractable component paths in a schema
    ListComponents {
        /// Input JSON Schema file
        input: PathBuf,
    },

    /// Generate a typed SDK project from converted schemas
    GenSdk {
        /// Target language for the generated SDK
        #[arg(short, long, value_enum)]
        language: SdkLanguage,

        /// Directory containing manifest.json and component schemas (output of `convert --output-dir`)
        #[arg(short, long)]
        schema: PathBuf,

        /// Package name (Java: "com.example.petstore", Python: "my-sdk")
        #[arg(short, long)]
        package: String,

        /// Output directory for the generated SDK project
        #[arg(short, long)]
        output: PathBuf,

        /// Initialize a git repository in the generated project
        #[arg(long, default_value_t = false)]
        git_init: bool,

        /// Build tool for the generated project
        #[arg(long, value_enum, default_value_t = BuildToolArg::Maven)]
        build_tool: BuildToolArg,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum TargetArg {
    OpenaiStrict,
    Gemini,
    Claude,
}

impl From<TargetArg> for Target {
    fn from(val: TargetArg) -> Self {
        match val {
            TargetArg::OpenaiStrict => Target::OpenaiStrict,
            TargetArg::Gemini => Target::Gemini,
            TargetArg::Claude => Target::Claude,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum ModeArg {
    Strict,
    Permissive,
}

impl From<ModeArg> for Mode {
    fn from(val: ModeArg) -> Self {
        match val {
            ModeArg::Strict => Mode::Strict,
            ModeArg::Permissive => Mode::Permissive,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum PolymorphismArg {
    #[value(name = "anyof")]
    AnyOf,
    Flatten,
}

impl From<PolymorphismArg> for PolymorphismStrategy {
    fn from(val: PolymorphismArg) -> Self {
        match val {
            PolymorphismArg::AnyOf => PolymorphismStrategy::AnyOf,
            PolymorphismArg::Flatten => PolymorphismStrategy::Flatten,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum OutputFormat {
    Pretty,
    Compact,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum SdkLanguage {
    Java,
    Python,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum BuildToolArg {
    Maven,
    Setuptools,
}

// ---------------------------------------------------------------------------
// Manifest types (#179)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: String,
    generated_at: String,
    source_schema: String,
    target: String,
    mode: String,
    components: Vec<ManifestComponent>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestComponent {
    name: String,
    pointer: String,
    schema_path: String,
    codec_path: String,
    dependency_count: usize,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing — logs go to stderr so stdout stays clean for JSON
    let log_level = if cli.verbose {
        LevelFilter::DEBUG
    } else {
        LevelFilter::WARN
    };
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_writer(std::io::stderr)
        .init();

    match cli.command {
        Commands::Convert {
            input,
            output,
            output_dir,
            codec: codec_path,
            target,
            mode,
            polymorphism,
            max_depth,
            recursion_limit,
            skip_components,
            format,
        } => {
            let schema = read_schema(&input)?;

            let mut options = ConvertOptions::default();
            options.target = target.into();
            options.mode = mode.into();
            options.polymorphism = polymorphism.into();
            options.max_depth = max_depth;
            options.recursion_limit = recursion_limit;
            options.skip_components = skip_components;

            if let Some(ref dir) = output_dir {
                // --output-dir mode: multi-file output with components
                handle_output_dir(&schema, &input, dir, &options, format)?;
            } else {
                // Single-file output mode (original behavior)
                let result = convert(&schema, &options)
                    .map_err(|e| anyhow::Error::from(e).context("Conversion failed"))?;

                // Warn if no codec file specified
                if codec_path.is_none() {
                    eprintln!(
                        "Warning: No codec file specified. You will not be able to rehydrate LLM outputs."
                    );
                }

                // Write converted schema
                write_json(&result.schema, output.as_ref(), format)?;

                // Write codec sidecar
                if let Some(path) = codec_path {
                    write_json(&result.codec, Some(&path), format)?;
                }

                // Report provider compat diagnostics (informational — transforms were applied)
                if !result.provider_compat_errors.is_empty() {
                    eprintln!("Provider compatibility diagnostics:");
                    for err in &result.provider_compat_errors {
                        eprintln!("- {}", err);
                    }
                }
            }
        }
        Commands::Rehydrate {
            input,
            codec,
            schema,
            output,
            format,
        } => {
            let data: serde_json::Value = {
                let raw = std::fs::read_to_string(&input)
                    .with_context(|| format!("Failed to read input file: {}", input.display()))?;

                // LLM output commonly has trailing characters (extra braces, whitespace).
                // Use serde_json's streaming deserializer to parse only the first valid
                // JSON value and ignore trailing garbage.
                let mut de = serde_json::Deserializer::from_str(&raw);
                serde_json::Value::deserialize(&mut de).with_context(|| {
                    format!("Failed to parse input data from: {}", input.display())
                })?
            };

            let codec_obj: Codec = {
                let file = File::open(&codec)
                    .with_context(|| format!("Failed to open codec file: {}", codec.display()))?;
                let reader = BufReader::new(file);
                serde_json::from_reader(reader)
                    .with_context(|| format!("Failed to parse codec from: {}", codec.display()))?
            };

            let original_schema: serde_json::Value = {
                let file = File::open(&schema)
                    .with_context(|| format!("Failed to open schema file: {}", schema.display()))?;
                let reader = BufReader::new(file);
                serde_json::from_reader(reader)
                    .with_context(|| format!("Failed to parse schema from: {}", schema.display()))?
            };

            let result = rehydrate(&data, &codec_obj, &original_schema)
                .map_err(|e| anyhow::Error::from(e).context("Rehydration failed"))?;

            for warning in &result.warnings {
                eprintln!("Warning: {}", warning.message);
            }

            write_json(&result.data, output.as_ref(), format)?;
        }
        Commands::Extract {
            input,
            pointer,
            output,
            format,
        } => {
            let schema = read_schema(&input)?;
            let extract_opts = ExtractOptions::default();
            let result = extract_component(&schema, &pointer, &extract_opts)
                .map_err(|e| anyhow::Error::from(e).context("Extraction failed"))?;

            write_json(&result.schema, output.as_ref(), format)?;
        }
        Commands::ListComponents { input } => {
            let schema = read_schema(&input)?;
            let components = list_components(&schema);
            for pointer in &components {
                println!("{}", pointer);
            }
        }
        Commands::GenSdk {
            language,
            schema,
            package,
            output,
            git_init,
            build_tool,
        } => {
            // Language-aware package name validation
            match language {
                SdkLanguage::Java => {
                    if !package
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '.' || c == '_')
                    {
                        anyhow::bail!(
                            "Invalid Java package name '{}': must contain only alphanumeric, dot, and underscore",
                            package
                        );
                    }
                }
                SdkLanguage::Python => {
                    // PEP 508: ^[a-zA-Z0-9]([a-zA-Z0-9._-]*[a-zA-Z0-9])?$
                    let valid = !package.is_empty()
                        && package.starts_with(|c: char| c.is_ascii_alphanumeric())
                        && package.ends_with(|c: char| c.is_ascii_alphanumeric())
                        && package
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
                    if !valid {
                        anyhow::bail!(
                            "Invalid Python package name '{}': must start and end with alphanumeric, \
                             contain only alphanumeric, hyphen, underscore, and dot (PEP 508)",
                            package
                        );
                    }
                }
            }

            // Derive artifact name from package
            let artifact_name = match language {
                SdkLanguage::Java => package
                    .trim_end_matches('.')
                    .rsplit('.')
                    .next()
                    .unwrap()
                    .to_string(),
                SdkLanguage::Python => package.clone(),
            };

            // Resolve build tool: use explicit flag or language default
            let resolved_build_tool = match (language, build_tool) {
                (_, BuildToolArg::Maven) => jsonschema_llm_codegen::BuildTool::Maven,
                (_, BuildToolArg::Setuptools) => jsonschema_llm_codegen::BuildTool::Setuptools,
            };

            let config = jsonschema_llm_codegen::SdkConfig {
                package,
                artifact_name,
                schema_dir: schema,
                output_dir: output,
                git_init,
                build_tool: resolved_build_tool,
            };

            jsonschema_llm_codegen::generate(&config).context("SDK generation failed")?;

            eprintln!(
                "SDK generated successfully at: {}",
                config.output_dir.display()
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read and parse a JSON Schema from a file path.
fn read_schema(input: &Path) -> Result<Value> {
    let file = File::open(input)
        .with_context(|| format!("Failed to open input file: {}", input.display()))?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader)
        .with_context(|| format!("Failed to parse schema from: {}", input.display()))
}

/// Handle `--output-dir` mode: convert all components and write to directory.
fn handle_output_dir(
    schema: &Value,
    input_path: &Path,
    output_dir: &Path,
    options: &ConvertOptions,
    format: OutputFormat,
) -> Result<()> {
    let extract_opts = ExtractOptions::default();
    let result = convert_all_components(schema, options, &extract_opts)
        .map_err(|e| anyhow::Error::from(e).context("Conversion failed"))?;

    // Create output directory
    fs::create_dir_all(output_dir).with_context(|| {
        format!(
            "Failed to create output directory: {}",
            output_dir.display()
        )
    })?;

    // Write full schema and codec at root
    write_json(
        &result.full.schema,
        Some(&output_dir.join("schema.json")),
        format,
    )?;
    write_json(
        &result.full.codec,
        Some(&output_dir.join("codec.json")),
        format,
    )?;

    // Report provider compat diagnostics
    if !result.full.provider_compat_errors.is_empty() {
        eprintln!("Provider compatibility diagnostics:");
        for err in &result.full.provider_compat_errors {
            eprintln!("- {}", err);
        }
    }

    // Write per-component files
    let mut manifest_components: Vec<ManifestComponent> = Vec::new();

    for (pointer, conv_result) in &result.components {
        let rel_dir = pointer_to_dir_path(pointer);
        let comp_dir = output_dir.join(&rel_dir);
        fs::create_dir_all(&comp_dir).with_context(|| {
            format!(
                "Failed to create component directory: {}",
                comp_dir.display()
            )
        })?;

        write_json(
            &conv_result.schema,
            Some(&comp_dir.join("schema.json")),
            format,
        )?;
        write_json(
            &conv_result.codec,
            Some(&comp_dir.join("codec.json")),
            format,
        )?;

        // Get dependency count by running extract_component independently
        let dep_count = extract_component(schema, pointer, &extract_opts)
            .map(|r| r.dependency_count)
            .unwrap_or(0);

        let name = pointer
            .rsplit('/')
            .next()
            .unwrap_or(pointer)
            .replace("~1", "/")
            .replace("~0", "~");

        manifest_components.push(ManifestComponent {
            name,
            pointer: pointer.clone(),
            schema_path: format!("{}/schema.json", rel_dir),
            codec_path: format!("{}/codec.json", rel_dir),
            dependency_count: dep_count,
        });
    }

    // Write component errors to stderr
    for (pointer, error) in &result.component_errors {
        eprintln!("Component error ({}): {}", pointer, error);
    }

    // Derive target/mode strings for manifest
    let target_str = serde_json::to_value(options.target)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "unknown".to_string());
    let mode_str = serde_json::to_value(options.mode)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "unknown".to_string());

    let source_name = input_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let manifest = Manifest {
        version: "1".to_string(),
        generated_at: Utc::now().to_rfc3339(),
        source_schema: source_name,
        target: target_str,
        mode: mode_str,
        components: manifest_components,
    };

    write_json(
        &manifest,
        Some(&output_dir.join("manifest.json")),
        OutputFormat::Pretty,
    )?;

    Ok(())
}

/// Convert a JSON Pointer to a relative directory path.
///
/// Strips the leading `#/` and uses the remaining segments as directory hierarchy.
/// Example: `#/$defs/Pet` → `$defs/Pet`
///          `#/components/schemas/User` → `components/schemas/User`
fn pointer_to_dir_path(pointer: &str) -> String {
    let stripped = pointer
        .strip_prefix("#/")
        .unwrap_or(pointer.strip_prefix('#').unwrap_or(pointer));
    // Sanitize: reject path traversal segments
    stripped
        .split('/')
        .filter(|seg| !seg.is_empty() && *seg != ".." && *seg != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn write_json<T: serde::Serialize>(
    val: &T,
    path: Option<&PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    let mut writer: Box<dyn Write> = if let Some(p) = path {
        let file = File::create(p)
            .with_context(|| format!("Failed to create output file: {}", p.display()))?;
        Box::new(BufWriter::new(file))
    } else {
        Box::new(BufWriter::new(io::stdout()))
    };

    match format {
        OutputFormat::Pretty => {
            serde_json::to_writer_pretty(&mut writer, val).context("Failed to write JSON")?;
        }
        OutputFormat::Compact => {
            serde_json::to_writer(&mut writer, val).context("Failed to write JSON")?;
        }
    }

    // Ensure trailing newline
    writeln!(writer).context("Failed to write trailing newline")?;

    Ok(())
}
