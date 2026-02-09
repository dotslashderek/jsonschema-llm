use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use jsonschema_llm_core::config::PolymorphismStrategy;
use jsonschema_llm_core::{convert, rehydrate, Codec, ConvertOptions, Target};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::PathBuf;
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
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output codec (rehydration metadata) file
        #[arg(long)]
        codec: Option<PathBuf>,

        /// Target LLM provider
        #[arg(short, long, value_enum, default_value_t = TargetArg::OpenaiStrict)]
        target: TargetArg,

        /// Polymorphism strategy
        #[arg(long, value_enum, default_value_t = PolymorphismArg::AnyOf)]
        polymorphism: PolymorphismArg,

        /// Max traversal depth for ref resolution
        #[arg(long, default_value_t = 50)]
        max_depth: usize,

        /// Recursion limit (cycles before breaking with placeholder)
        #[arg(long, default_value_t = 3)]
        recursion_limit: usize,

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

        /// Output rehydrated JSON file (defaults to stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Pretty)]
        format: OutputFormat,
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
            codec: codec_path,
            target,
            polymorphism,
            max_depth,
            recursion_limit,
            format,
        } => {
            let file = File::open(&input)
                .with_context(|| format!("Failed to open input file: {}", input.display()))?;
            let reader = BufReader::new(file);
            let schema: serde_json::Value = serde_json::from_reader(reader)
                .with_context(|| format!("Failed to parse schema from: {}", input.display()))?;

            // All fields set explicitly; clippy enforces exhaustiveness
            let options = ConvertOptions {
                target: target.into(),
                polymorphism: polymorphism.into(),
                max_depth,
                recursion_limit,
            };

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
        }
        Commands::Rehydrate {
            input,
            codec,
            output,
            format,
        } => {
            let data: serde_json::Value = {
                let file = File::open(&input)
                    .with_context(|| format!("Failed to open input file: {}", input.display()))?;
                let reader = BufReader::new(file);
                serde_json::from_reader(reader).with_context(|| {
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

            let result = rehydrate(&data, &codec_obj)
                .map_err(|e| anyhow::Error::from(e).context("Rehydration failed"))?;

            for warning in &result.warnings {
                eprintln!("Warning: {}", warning.message);
            }

            write_json(&result.data, output.as_ref(), format)?;
        }
    }

    Ok(())
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
