use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use flox_rust_sdk::models::environment::generations::AllGenerationsMetadata;
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::models::manifest::typed::Manifest;

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "Rust tasks for the Flox project")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate JSON schemas for Flox data structures
    GenerateSchemas {
        /// The schema to generate (if not specified, generates all schemas)
        #[arg(value_enum)]
        schema: Option<SchemaType>,
    },
}

#[derive(Clone, ValueEnum)]
enum SchemaType {
    /// Generate manifest schema
    Manifest,
    /// Generate lockfile schema
    Lockfile,
    /// Generate generations metadata schema
    GenerationsMetadata,
}

fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();

    match cli.command {
        Commands::GenerateSchemas { schema } => {
            if let Some(schema_type) = schema {
                generate_schema(schema_type)?;
            } else {
                // Generate all schemas
                generate_schema(SchemaType::Manifest)?;
                generate_schema(SchemaType::Lockfile)?;
                generate_schema(SchemaType::GenerationsMetadata)?;
            }
        },
    }

    Ok(())
}

fn generate_schema(schema_type: SchemaType) -> Result<()> {
    let (schema, filename) = match schema_type {
        SchemaType::Manifest => (schemars::schema_for!(Manifest), "manifest-v1.schema.json"),
        SchemaType::Lockfile => (schemars::schema_for!(Lockfile), "lockfile-v1.schema.json"),
        SchemaType::GenerationsMetadata => (
            schemars::schema_for!(AllGenerationsMetadata),
            "generations-metadata-v2.schema.json",
        ),
    };

    let schema_dir = project_root().join("schemas");
    fs::create_dir_all(&schema_dir).context("Failed to create schemas directory")?;
    let output_path = schema_dir.join(filename);

    let mut schema_file =
        File::create(&output_path).context("Failed to create schema output file")?;

    writeln!(&mut schema_file, "{:#}", schema.as_value())
        .context("Failed to write schema to file")?;

    println!("Generated schema: {}", output_path.display());

    Ok(())
}

fn project_root() -> PathBuf {
    // Slightly hacky since we cant read the workspace dir directly:
    // <https://github.com/rust-lang/cargo/issues/3946>
    Path::new(&env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Cargo.toml must be in a directory")
        .to_path_buf()
}
