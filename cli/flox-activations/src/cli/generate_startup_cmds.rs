use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::path::PathBuf;

use anyhow::bail;
use clap::Args;
use log::debug;

use super::{join_dir_list, separate_dir_list};

#[derive(Debug, Args)]
pub struct StartupCmdsArgs {
    /// Which shell syntax to emit
    #[arg(short, long, value_name = "SHELL")]
    pub shell: String,
    /// The path to the runtime directory keeping activation data.
    #[arg(long, value_name = "PATH")]
    pub runtime_dir: PathBuf,
    /// The `_flox_activate_tracelevel` variable
    #[arg(long, value_name = "STRING")]
    pub trace_level: u8,
    /// The path to the activation interpreter.
    #[arg(long, value_name = "$_activate_d")]
    pub activate_d: String,
    /// The path to the rendered environment.
    #[arg(long, value_name = "FLOX_ENV")]
    pub flox_env: String,
    /// Whether to run activation with only profile scripts.
    #[arg(long, value_name = "BOOL")]
    pub profile_only: String,
    /// The environment's cache.
    #[arg(long, value_name = "_FLOX_ENV_CACHE")]
    pub env_cache: String,
    /// The environment's project directory.
    #[arg(long, value_name = "_FLOX_ENV_PROJECT")]
    pub project_dir: PathBuf,
    /// The environment's description.
    #[arg(long, value_name = "_FLOX_ENV_DESCRIPTION")]
    pub env_description: String,
    /// Whether the activation is in-place or not.
    #[arg(long, value_name = "BOOL")]
    pub in_place: String,
    /// Whether the activation has a tty.
    #[arg(long, value_name = "BOOL")]
    pub has_tty: String,
    /// Whether this shell has previously sourced its RC file
    #[arg(long, value_name = "BOOL")]
    pub previously_sourced_rc: String,
    /// Whether this shell is in the process of sourcing its RC file
    #[arg(long, value_name = "BOOL")]
    pub sourcing_rc: String,
    /// The path to the `sed` executable
    #[arg(long, value_name = "STRING")]
    pub sed: String,
}

impl StartupCmdsArgs {
    pub fn handle(&self) -> Result<(), anyhow::Error> {
        let mut stdout = std::io::stdout();
        self.handle_inner(&mut stdout)?;
        Ok(())
    }

    pub fn handle_inner(&self, output: &mut impl Write) -> Result<(), anyhow::Error> {
        if self.shell != "bash" {
            bail!("only bash is supported at the moment");
        }
        if self.trace_level > 2 {
            writeln!(output, "set -x")?;
        }
        if self.profile_only != "true" {
            // Login inputs
            let bashrc_exists = dirs::home_dir().map(|path| path.exists()).unwrap_or(false);
            let currently_sourcing = self.sourcing_rc == "true";
            let previously_sourced = self.previously_sourced_rc == "true";
            let is_in_place = self.in_place == "true";

            let should_source = if bashrc_exists {
                if is_in_place {
                    !(previously_sourced || currently_sourcing)
                } else {
                    true
                }
            } else {
                false
            };

            if should_source {
                writeln!(output, "export _flox_sourcing_rc=true;")?;
                writeln!(output, "source ~/.bashrc;")?;
                writeln!(output, "unset _flox_sourcing_rc;")?;
                writeln!(output, "export _flox_sourced_rc=true;")?;
            }

            writeln!(output, "export FLOX_ENV='{}';", &self.flox_env)?;
        }

        Ok(())
    }
}
