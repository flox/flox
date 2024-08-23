use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;

#[derive(Debug, Bpaf, Clone)]
pub enum EnvironmentOrGlobalSelect {
    /// Update the global base catalog
    #[bpaf(long("global"))]
    Global,
    #[allow(unused)]
    Environment(#[bpaf(external(environment_select))] EnvironmentSelect),
}

impl Default for EnvironmentOrGlobalSelect {
    fn default() -> Self {
        EnvironmentOrGlobalSelect::Environment(Default::default())
    }
}

// Update the global base catalog or an environment's base catalog
#[derive(Bpaf, Clone)]
pub struct Update {
    #[bpaf(external(environment_or_global_select), fallback(Default::default()))]
    _environment_or_global: EnvironmentOrGlobalSelect,

    #[bpaf(positional("inputs"), hide)]
    _inputs: Vec<String>,
}

impl Update {
    #[instrument(name = "update", skip_all)]
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("update");
        bail!("'flox update' has been removed.\n\nTo upgrade packages, run 'flox upgrade'. See flox-upgrade(1) for more.");
    }
}
