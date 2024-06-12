use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::UpdateResult;
use flox_rust_sdk::models::lockfile::{Input, LockedManifestPkgdb, TypedLockedManifestPkgdb};
use flox_rust_sdk::models::pkgdb::{self, ScrapeError};
use tracing::instrument;

use super::{environment_select, ConcreteEnvironment, EnvironmentSelect};
use crate::commands::environment_description;
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::message::{self};

#[derive(Debug, Bpaf, Clone)]
pub enum EnvironmentOrGlobalSelect {
    /// Update the global base catalog
    #[bpaf(long("global"))]
    Global,
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
    environment_or_global: EnvironmentOrGlobalSelect,

    #[bpaf(positional("inputs"), hide)]
    inputs: Vec<String>,
}

impl Update {
    #[instrument(name = "update", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        if flox.catalog_client.is_some() {
            bail!("'flox update' has been removed. To upgrade packages, run 'flox upgrade'. See flox-upgrade(1) for more.");
        }

        subcommand_metric!("update");

        let (old_lockfile, new_lockfile, global, description) = match self.environment_or_global {
            EnvironmentOrGlobalSelect::Environment(ref environment_select) => {
                let span = tracing::info_span!("update_local");
                let _guard = span.enter();

                let concrete_environment =
                    environment_select.detect_concrete_environment(&flox, "Update")?;

                let description = Some(environment_description(&concrete_environment)?);
                let UpdateResult {
                    new_lockfile,
                    old_lockfile,
                    ..
                } = Dialog {
                    message: "Updating environment...",
                    help_message: None,
                    typed: Spinner::new(|| self.update_manifest(flox, concrete_environment)),
                }
                .spin()?;

                (
                    old_lockfile
                        .map(TypedLockedManifestPkgdb::try_from)
                        .transpose()?,
                    TypedLockedManifestPkgdb::try_from(new_lockfile)?,
                    false,
                    description,
                )
            },
            EnvironmentOrGlobalSelect::Global => {
                let span = tracing::info_span!("update_global");
                let _guard = span.enter();

                let UpdateResult {
                    new_lockfile,
                    old_lockfile,
                    ..
                } = Dialog {
                    message: "Updating global-manifest...",
                    help_message: None,
                    typed: Spinner::new(|| {
                        LockedManifestPkgdb::update_global_manifest(&flox, self.inputs)
                    }),
                }
                .spin()?;

                (
                    old_lockfile
                        .map(TypedLockedManifestPkgdb::try_from)
                        .transpose()?,
                    TypedLockedManifestPkgdb::try_from(new_lockfile)?,
                    true,
                    None,
                )
            },
        };

        if let Some(ref old_lockfile) = old_lockfile {
            if new_lockfile.registry().inputs == old_lockfile.registry().inputs {
                if global {
                    message::plain("ℹ️  All global inputs are up-to-date.");
                } else {
                    message::plain(format!(
                        "ℹ️  All inputs are up-to-date in environment {}.",
                        description.as_ref().unwrap()
                    ));
                }

                return Ok(());
            }
        }

        let mut inputs_to_scrape: Vec<&Input> = vec![];

        for (input_name, new_input) in &new_lockfile.registry().inputs {
            let old_input = old_lockfile
                .as_ref()
                .and_then(|old| old.registry().inputs.get(input_name));
            match old_input {
                // unchanged input
                Some(old_input) if old_input == new_input => continue, // dont need to scrape
                // updated input
                Some(_) if global => {
                    message::plain(format!("⬆️  Updated global input '{}'.", input_name))
                },
                Some(_) => message::plain(format!(
                    "⬆️  Updated input '{}' in environment {}.",
                    input_name,
                    description.as_ref().unwrap()
                )),
                // new input
                None if global => {
                    message::plain(format!("🔒️  Locked global input '{}'.", input_name))
                },
                None => message::plain(format!(
                    "🔒️  Locked input '{}' in environment {}.",
                    input_name,
                    description.as_ref().unwrap(),
                )),
            }
            inputs_to_scrape.push(new_input);
        }

        if let Some(old_lockfile) = old_lockfile {
            for input_name in old_lockfile.registry().inputs.keys() {
                if !new_lockfile.registry().inputs.contains_key(input_name) {
                    if global {
                        message::deleted(format!(
                            "Removed unused input '{}' from global lockfile.",
                            input_name
                        ));
                    } else {
                        message::deleted(format!(
                            "Removed unused input '{}' from lockfile for environment {}.",
                            input_name,
                            description.as_ref().unwrap()
                        ));
                    }
                }
            }
        }

        if inputs_to_scrape.is_empty() {
            return Ok(());
        }

        let span = tracing::info_span!("scrape");
        let _guard = span.enter();
        let results: Vec<Result<(), ScrapeError>> = Dialog {
            message: "Generating databases for updated inputs...",
            help_message: (inputs_to_scrape.len() > 1).then_some("This may take a while."),
            typed: Spinner::new(|| {
                inputs_to_scrape
                    .iter()
                    .map(|input| pkgdb::scrape_input(&input.from))
                    .collect()
            }),
        }
        .spin();
        drop(_guard);

        for result in results {
            result?;
        }

        Ok(())
    }

    fn update_manifest(
        &self,
        flox: Flox,
        concrete_environment: ConcreteEnvironment,
    ) -> Result<UpdateResult> {
        let mut environment = concrete_environment.into_dyn_environment();

        Ok(environment.update(&flox, self.inputs.clone())?)
        // .context("updating environment failed")
    }
}
