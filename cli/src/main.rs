use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "jsonschema-llm")]
#[command(about = "Convert any JSON Schema into an LLM-compatible structured output schema")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Convert a JSON Schema to an LLM-compatible schema
    Convert {
        /// Input JSON Schema file
        input: PathBuf,

        /// Output converted schema file
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output codec (rehydration metadata) file
        #[arg(long)]
        codec: Option<PathBuf>,

        /// Target LLM provider
        #[arg(short, long, default_value = "openai-strict")]
        target: String,
    },

    /// Rehydrate LLM output back to the original schema shape
    Rehydrate {
        /// LLM output JSON file
        input: PathBuf,

        /// Codec file from conversion
        #[arg(long)]
        codec: PathBuf,

        /// Output rehydrated JSON file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Convert {
            input,
            output,
            codec,
            target,
        } => {
            eprintln!("Converting {} for target '{}'...", input.display(), target);
            // TODO: implement
            eprintln!("Not yet implemented. Coming soon!");
        }
        Commands::Rehydrate {
            input,
            codec,
            output,
        } => {
            eprintln!(
                "Rehydrating {} with codec {}...",
                input.display(),
                codec.display()
            );
            // TODO: implement
            eprintln!("Not yet implemented. Coming soon!");
        }
    }
}
