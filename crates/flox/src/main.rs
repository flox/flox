use anyhow::Result;
use flox_rust_sdk::environment::build_flox_env;
use log::{debug, info};
use std::env;
use std::fmt::Debug;
use std::process::ExitStatus;

use tokio::process::Command;

mod build;
mod config;
mod utils;
pub static FLOX_SH: &str = env!("FLOX_SH");
static FLOX_VERSION: &str = env!("FLOX_VERSION");

mod commands {
    use std::{os::unix::process, str::FromStr};

    use anyhow::Result;
    use bpaf::Bpaf;
    use flox_rust_sdk::flox::Flox;
    use flox_rust_sdk::prelude::{Channel, ChannelRegistry};
    use tempfile::TempDir;

    use crate::FLOX_VERSION;

    use self::environment::EnvironmentArgs;
    use self::package::PackageArgs;

    #[derive(Bpaf)]
    #[bpaf(options, version(FLOX_VERSION))]
    pub struct FloxArgs {
        verbose: bool,

        debug: bool,

        #[bpaf(external(commands))]
        command: Commands,

        #[bpaf(positional)]
        nix_args: Vec<String>,
    }

    impl FloxArgs {
        /// Initialize the command line by creating an initial FloxBuilder
        pub async fn handle(&self, config: crate::config::Config) -> Result<()> {
            // prepare a temp dir for the run:
            let process_dir = config.flox.cache_dir.join("process");
            tokio::fs::create_dir_all(&process_dir).await?;

            // `temp_dir` will automatically be removed from disk when the function returns
            let temp_dir = TempDir::new_in(process_dir)?;

            let mut channels = ChannelRegistry::default();
            channels.register_channel("flox", Channel::from_str("github:flox/floxpkgs")?);
            channels.register_channel("nixpkgs", Channel::from_str("github:flox/nixpkgs/stable")?);

            // generate these dynamically based on <?>
            channels.register_channel(
                "nixpkgs-stable",
                Channel::from_str("github:flox/nixpkgs/stable")?,
            );
            channels.register_channel(
                "nixpkgs-staging",
                Channel::from_str("github:flox/nixpkgs/staging")?,
            );
            channels.register_channel(
                "nixpkgs-unstable",
                Channel::from_str("github:flox/nixpkgs/unstable")?,
            );

            let flox = Flox {
                collect_metrics: config.flox.allow_telemetry.unwrap_or_default(),
                cache_dir: config.flox.cache_dir,
                data_dir: config.flox.data_dir,
                config_dir: config.flox.config_dir,
                channels: channels,
                temp_dir: temp_dir.path().to_path_buf(),
            };

            match self.command {
                // Commands::Support(ref f) => f.run(self).await?,
                // Commands::Build(ref f) => f.run(&self).await?,
                Commands::Package(ref package) => {
                    package.handle(flox, self.verbose, self.debug).await?
                }
                Commands::Environment(ref environment) => environment.handle(flox).await?,
            }
            Ok(())
        }
    }

    /// Transparent separation of different categories of commands
    #[derive(Bpaf)]
    pub enum Commands {
        Package(
            #[bpaf(external(package::package_args))]
            #[bpaf(group_help("Development Commands"))]
            PackageArgs,
        ),
        Environment(
            #[bpaf(external(environment::environment_args))]
            #[bpaf(group_help("Environment Commands"))]
            EnvironmentArgs,
        ),
    }

    mod package {
        use anyhow::Result;
        use bpaf::Bpaf;
        use flox_rust_sdk::{flox::Flox, nix::command_line::NixCommandLine, prelude::Stability};

        use crate::run_in_flox;

        use self::build::BuildArgs;

        #[derive(Bpaf)]
        pub struct PackageArgs {
            stability: Option<Stability>,

            #[bpaf(external(package_commands))]
            command: PackageCommands,
        }

        impl PackageArgs {
            async fn forward(
                &self,
                command: impl ToString,
                mut passthru: Vec<String>,
                verbose: bool,
                debug: bool,
            ) -> Result<()> {
                let mut args = vec![];

                if verbose {
                    args.push("--verbose".to_string())
                }

                if debug {
                    args.push("--debug".to_string())
                }

                args.push(command.to_string());
                if let Some(ref stability) = self.stability {
                    args.append(&mut vec!["--stability".to_string(), stability.to_string()]);
                }

                args.append(&mut passthru);
                let status = run_in_flox(&args).await?;

                // Todo: add exit code error
                Ok(())
            }

            pub async fn handle(&self, flox: Flox, verbose: bool, debug: bool) -> Result<()> {
                match &self.command {
                    PackageCommands::Build(BuildArgs { installable }) => {
                        flox.package(
                            installable.clone().into(),
                            self.stability.clone().unwrap_or_default(),
                        )
                        .build::<NixCommandLine>()
                        .await?
                    }
                    PackageCommands::Develop { passthru } => {
                        self.forward("develop", passthru.clone(), verbose, debug)
                            .await?
                    }
                    PackageCommands::Publish { passthru } => {
                        self.forward("publish", passthru.clone(), verbose, debug)
                            .await?
                    }
                    PackageCommands::Run { passthru } => {
                        self.forward("run", passthru.clone(), verbose, debug)
                            .await?
                    }
                    PackageCommands::Shell { passthru } => {
                        self.forward("shell", passthru.clone(), verbose, debug)
                            .await?
                    }
                }

                Ok(())
            }
        }

        #[derive(Bpaf, Clone)]
        pub enum PackageCommands {
            /// build package from current project
            #[bpaf(command)]
            Build(#[bpaf(external(build::build_args))] build::BuildArgs),

            /// launch development shell for current project
            #[bpaf(command)]
            Develop { passthru: Vec<String> },
            /// build and publish project to flox channel
            #[bpaf(command)]
            Publish { passthru: Vec<String> },
            /// run app from current project
            #[bpaf(command)]
            Run { passthru: Vec<String> },
            /// run a shell in which the current project is available
            #[bpaf(command)]
            Shell { passthru: Vec<String> },
        }

        mod build {
            use bpaf::Bpaf;
            #[derive(Bpaf, Clone)]
            pub struct BuildArgs {
                #[bpaf(positional("INSTALLABLE"))]
                pub installable: String,
            }
        }
    }

    mod environment {
        use anyhow::Result;
        use bpaf::Bpaf;
        use flox_rust_sdk::flox::Flox;
        use flox_rust_sdk::nix::command_line::NixCommandLine;
        use std::path::PathBuf;

        use self::install::InstallArgs;
        use self::remove::RemoveArgs;

        #[derive(Bpaf)]
        pub struct EnvironmentArgs {
            /// path to environment. Note: this will be changed to an environment name
            #[bpaf(short('e'))]
            pub environment: PathBuf,
            #[bpaf(external(environment_commands))]
            command: EnvironmentCommands,
        }

        impl EnvironmentArgs {
            pub async fn handle(&self, flox: Flox) -> Result<()> {
                match &self.command {
                    EnvironmentCommands::List => {
                        flox.environment(self.environment.clone())
                            .list::<NixCommandLine>()
                            .await?
                    }
                    EnvironmentCommands::Edit => {
                        flox.environment(self.environment.clone())
                            .edit::<NixCommandLine>()
                            .await?
                    }
                    EnvironmentCommands::Install(InstallArgs { package }) => {
                        flox.environment(self.environment.clone())
                            .install::<NixCommandLine>(package)
                            .await?
                    }
                    EnvironmentCommands::Remove(RemoveArgs { package }) => {
                        flox.environment(self.environment.clone())
                            .remove::<NixCommandLine>(package)
                            .await?
                    }
                }

                Ok(())
            }
        }

        #[derive(Bpaf, Clone)]
        pub enum EnvironmentCommands {
            /// list packages installed in an environment
            #[bpaf(command)]
            List,
            /// edit declarative environment configuration
            #[bpaf(command)]
            Edit,
            /// install a package into an environment
            #[bpaf(command)]
            Install(#[bpaf(external(install::install_args))] install::InstallArgs),
            /// remove packages from an environment
            #[bpaf(command)]
            Remove(#[bpaf(external(remove::remove_args))] remove::RemoveArgs),
        }

        mod install {
            use bpaf::Bpaf;
            #[derive(Bpaf, Clone)]
            pub struct InstallArgs {
                #[bpaf(positional("PACKAGE"))]
                pub package: String,
            }
        }

        mod remove {
            use bpaf::Bpaf;
            #[derive(Bpaf, Clone)]
            pub struct RemoveArgs {
                #[bpaf(positional("PACKAGE"))]
                pub package: String,
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    if crate::config::Config::preview_enabled()? {
        run_rust_flox().await?;
    } else {
        info!("`FLOX_PREVIEW` unset or not \"true\", falling back to legacy flox");
        run_in_flox(&env::args_os().collect::<Vec<_>>()[1..]).await?;
    }

    Ok(())
}

async fn run_rust_flox() -> Result<()> {
    let args = commands::flox_args().run();
    args.handle(config::Config::parse()?).await?;
    Ok(())
}

pub async fn run_in_flox(args: &[impl AsRef<std::ffi::OsStr> + Debug]) -> Result<ExitStatus> {
    debug!("Running in flox with arguments: {:?}", args);
    let status = Command::new(FLOX_SH)
        .args(args)
        .envs(&build_flox_env())
        .spawn()
        .expect("failed to spawn flox")
        .wait()
        .await?;

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_flox_help() {
        // TODO check the output
        assert_eq!(run_in_flox(&["--help"]).await.unwrap().code().unwrap(), 0,)
    }
}
