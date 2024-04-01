use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::EnvironmentError;
use indoc::formatdoc;
use itertools::Itertools;
use log::debug;
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::commands::{
    ensure_floxhub_token,
    environment_description,
    ConcreteEnvironment,
    EnvironmentSelectError,
};
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::message;

// Uninstall installed packages from an environment
#[derive(Bpaf, Clone)]
pub struct Uninstall {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// The install IDs of the packages to remove
    #[bpaf(positional("packages"), some("Must specify at least one package"))]
    packages: Vec<String>,
}

impl Uninstall {
    #[instrument(name = "uninstall", fields(packages), skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        subcommand_metric!("uninstall");

        // Vec<T> doesn't implement tracing::Value, so you have to join the strings
        // yourself.
        tracing::Span::current().record("packages", self.packages.iter().join(","));

        debug!(
            "uninstalling packages [{}] from {:?}",
            self.packages.as_slice().join(", "),
            self.environment
        );
        let concrete_environment = match self
            .environment
            .detect_concrete_environment(&flox, "Uninstall from")
        {
            Ok(concrete_environment) => concrete_environment,
            Err(EnvironmentSelectError::Environment(
                ref e @ EnvironmentError::DotFloxNotFound(ref dir),
            )) => {
                bail!(formatdoc! {"
                {e}

                Create an environment with 'flox init --dir {}'", dir.to_string_lossy()
                })
            },
            Err(e @ EnvironmentSelectError::EnvNotFoundInCurrentDirectory) => {
                bail!(formatdoc! {"
                {e}

                Create an environment with 'flox init' or uninstall packages from an environment found elsewhere with 'flox uninstall {} --dir <path>'",
                self.packages.join(" ")})
            },
            Err(e) => Err(e)?,
        };

        // Ensure the user is logged in for the following remote operations
        if let ConcreteEnvironment::Remote(_) = concrete_environment {
            ensure_floxhub_token(&mut flox).await?;
        };

        let description = environment_description(&concrete_environment)?;
        let mut environment = concrete_environment.into_dyn_environment();

        let _ = Dialog {
            message: &format!("Uninstalling packages from environment {description}..."),
            help_message: None,
            typed: Spinner::new(|| environment.uninstall(self.packages.clone(), &flox)),
        }
        .spin()?;

        // Note, you need two spaces between this emoji and the package name
        // otherwise they appear right next to each other.
        self.packages.iter().for_each(|p| {
            message::deleted(format!("'{p}' uninstalled from environment {description}"))
        });
        Ok(())
    }
}
