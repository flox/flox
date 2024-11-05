use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::providers::build::FloxBuildMk;
use flox_rust_sdk::providers::publish::{
    check_build_metadata,
    check_environment_metadata,
    Publish,
    PublishProvider,
};
use indoc::indoc;
use log::debug;
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::commands::{ensure_floxhub_token, ConcreteEnvironment};
use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::message;

#[allow(unused)] // remove when we implement the command
#[derive(Bpaf, Clone)]
pub struct PublishCmd {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(external(subcommand_or_build_targets))]
    subcommand_or_targets: SubcommandOrBuildTargets,
}

#[derive(Debug, Bpaf, Clone)]
enum SubcommandOrBuildTargets {
    BuildTarget {
        /// The package to build.
        /// Corresponds to entries in the 'build' table in the environment's manifest.toml.
        /// If not specified, all packages are built.
        #[bpaf(positional("package"))]
        target: String,
    },
}

impl PublishCmd {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        if !config.features.unwrap_or_default().publish {
            message::plain("🚧 👷 heja, a new command is in construction here, stay tuned!");
            bail!("'publish' feature is not enabled.");
        }

        match self.subcommand_or_targets {
            SubcommandOrBuildTargets::BuildTarget { target } => {
                let env = self
                    .environment
                    .detect_concrete_environment(&flox, "Clean build files of")?;

                Self::publish(flox, env, target).await
            },
        }
    }

    #[instrument(name = "publish", skip_all, fields(package))]
    async fn publish(mut flox: Flox, env: ConcreteEnvironment, package: String) -> Result<()> {
        subcommand_metric!("publish");

        if let ConcreteEnvironment::Remote(_) = &env {
            bail!("Cannot publish from a remote environment");
        };

        let mut env = env.into_dyn_environment();

        if !check_package(&env.lockfile(&flox)?, &package)? {
            bail!("Package '{}' not found in environment", package);
        }

        let (env_meta, build_meta) = (
            check_environment_metadata(&flox, &*env).unwrap(),
            check_build_metadata(&*env, &package, &flox.system).unwrap(),
        );

        let publish_provider = PublishProvider::<&FloxBuildMk> {
            build_meta,
            env_meta,
            _builder: None,
        };

        ensure_floxhub_token(&mut flox).await?;
        let token = flox.floxhub_token.as_ref().unwrap();

        debug!("publishing package: {}", &package);
        match publish_provider.publish(&flox.catalog_client, token).await {
            Ok(_) => message::created("Package published successfully"),
            Err(e) => bail!("Failed to publish package: {}", e.to_string()),
        }

        // let base_dir = env.parent_path()?;
        // let flox_env = env.activation_path(&flox)?;

        // let packages_to_build = available_packages(&env.lockfile(&flox)?, package)?;

        // let builder = FloxBuildMk;
        // let output = builder.build(&flox, &base_dir, &flox_env, &packages_to_build)?;

        // for message in output {
        //     match message {
        //         Output::Stdout(line) => println!("{line}"),
        //         Output::Stderr(line) => eprintln!("{line}"),
        //         Output::Exit(status) if status.success() => {
        //             message::created("Build completed successfully");
        //             break;
        //         },
        //         Output::Exit(status) => {
        //             bail!("Build failed with status: {status}");
        //         },
        //     }
        // }

        Ok(())
    }
}

fn check_package(lockfile: &Lockfile, package: &str) -> Result<bool> {
    let environment_packages = &lockfile.manifest.build;

    if environment_packages.is_empty() {
        bail!(indoc! {"
        No builds found.

        Add a build by modifying the '[build]' section of the manifest with 'flox edit'
        "});
    }

    if !environment_packages.contains_key(package) {
        bail!("Package '{}' not found in environment", package);
    }

    Ok(true)
}
