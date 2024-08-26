use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::message;

#[allow(unused)] // remove when we implement the command
#[derive(Bpaf, Clone)]
pub struct Build {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Whether to print logs to stderr during build.
    /// Logs are always written to <TBD>
    #[bpaf(short('L'), long)]
    build_logs: bool,

    /// The package to build, corresponds to the entries in
    /// the 'build' table in the environment's manifest.toml.
    /// If not specified, all packages are built.
    #[bpaf(positional("build"))]
    package: Vec<String>,
}

impl Build {
    #[instrument(name = "build", skip_all)]
    pub async fn handle(self, _config: Config, _flox: Flox) -> Result<()> {
        subcommand_metric!("build");

        message::plain("🚧 👷 heja, a new command is in construction here, stay tuned!");
        Ok(())
    }
}
