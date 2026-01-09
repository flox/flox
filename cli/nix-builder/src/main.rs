use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};

mod builder;

/// Configuration read from environment variables, matching builder.pl's interface.
#[derive(Debug, Clone)]
pub struct Config {
    /// Path to Nix attributes JSON file (required)
    pub nix_attrs_json_file: PathBuf,
    /// Output directory (set by Nix)
    pub out: PathBuf,
    /// Space-separated paths to link (defaults to "/")
    pub paths_to_link: Vec<String>,
    /// Additional path prefix (defaults to "")
    pub extra_prefix: String,
    /// Collision handling: 0=error, 1=warn, 2=silent (defaults to 0)
    pub ignore_collisions: u8,
    /// Whether to check collision contents: 0 or 1 (defaults to 0)
    pub check_collision_contents: bool,
}

impl Config {
    /// Parse configuration from environment variables.
    fn from_env() -> Result<Self> {
        let nix_attrs_json_file = env::var("NIX_ATTRS_JSON_FILE")
            .context("missing required environment variable: NIX_ATTRS_JSON_FILE")?;
        let nix_attrs_json_file = PathBuf::from(nix_attrs_json_file);

        if !nix_attrs_json_file.exists() {
            bail!(
                "NIX_ATTRS_JSON_FILE does not exist: {}",
                nix_attrs_json_file.display()
            );
        }

        let out = env::var("out").context("missing required environment variable: out")?;
        let out = PathBuf::from(out);

        let paths_to_link = env::var("pathsToLink")
            .unwrap_or_else(|_| "/".to_string())
            .split_whitespace()
            .map(String::from)
            .collect();

        let extra_prefix = env::var("extraPrefix").unwrap_or_default();

        let ignore_collisions = env::var("ignoreCollisions")
            .unwrap_or_else(|_| "0".to_string())
            .parse::<u8>()
            .context("ignoreCollisions must be 0, 1, or 2")?;

        if ignore_collisions > 2 {
            bail!(
                "ignoreCollisions must be 0, 1, or 2, got: {}",
                ignore_collisions
            );
        }

        let check_collision_contents = env::var("checkCollisionContents")
            .unwrap_or_else(|_| "0".to_string())
            .parse::<u8>()
            .context("checkCollisionContents must be 0 or 1")?;

        if check_collision_contents > 1 {
            bail!(
                "checkCollisionContents must be 0 or 1, got: {}",
                check_collision_contents
            );
        }

        let check_collision_contents = check_collision_contents == 1;

        Ok(Config {
            nix_attrs_json_file,
            out,
            paths_to_link,
            extra_prefix,
            ignore_collisions,
            check_collision_contents,
        })
    }
}

fn run() -> Result<()> {
    let config = Config::from_env().context("failed to parse configuration from environment")?;

    builder::build_env(config)?;

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Print the error chain using the message.rs emoji style
            let err_str = err
                .chain()
                .skip(1)
                .fold(err.to_string(), |acc, cause| format!("{acc}: {cause}"));

            eprintln!("❌ ERROR: {}", err_str);
            ExitCode::FAILURE
        },
    }
}
