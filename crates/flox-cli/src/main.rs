use anyhow::Result;
use clap::Parser;
use flox_rust_sdk::environment::build_flox_env;
use log::{debug, error, info, warn};
use std::env;
use std::fmt::Debug;
use std::process::{exit, ExitStatus};
use tokio::process::Command;

mod build;
mod config;
mod utils;
pub static FLOX_SH: &str = env!("FLOX_SH");
mod commands {
    use anyhow::Result;
    use clap::{Parser, Subcommand};
    use flox_rust_sdk::prelude::FloxBuilder;

    use forward::{Forward, ForwardTo};

    use self::package::PackageArgs;

    #[derive(Parser)]
    #[clap(author, version, about, long_about = None)]
    pub struct FloxArgs {
        #[clap(long)]
        verbose: bool,

        #[clap(long)]
        debug: bool,

        #[clap(subcommand)]
        command: FloxCommands,

        #[clap(last = true)]
        nix_args: Vec<String>,
    }

    impl FloxArgs {
        pub async fn handle(&self) -> Result<()> {
            match self.command {
                FloxCommands::Support(ref f) => f.run(self).await?,
                FloxCommands::Build(ref f) => f.run(&self).await?,
                FloxCommands::Package(ref package) => package.handle(self).await?,
            }
            Ok(())
        }

        fn flox(&self) -> FloxBuilder {
            FloxBuilder::default()
        }
    }

    #[derive(Subcommand)]
    pub enum FloxCommands {
        /// allow explicitly forwarding to legacy flox
        Support(Forward<LegacyFlox>),
        Build(Forward<LegacyFloxBuild>),
        Package(PackageArgs),
    }

    impl ForwardTo for FloxCommands {
        /// any unknown commands to flox that are not subcommands knwo to flox
        const COMMAND: &'static [&'static str] = &[""];
    }

    pub struct LegacyFlox;
    impl ForwardTo for LegacyFlox {
        /// any unknown commands to flox that are not subcommands knwo to flox
        const COMMAND: &'static [&'static str] = &[""];
    }

    pub struct LegacyFloxBuild;
    impl ForwardTo for LegacyFloxBuild {
        const COMMAND: &'static [&'static str] = &["build"];
    }

    mod package {
        use anyhow::Result;
        use clap::{Parser, Subcommand};

        use self::build::BuildArgs;

        use super::FloxArgs;

        #[derive(Parser)]
        pub struct PackageArgs {
            #[clap(subcommand)]
            pub command: PackageCommands,
        }

        impl PackageArgs {
            pub async fn handle(&self, root_args: &FloxArgs) -> Result<()> {
                let flox = root_args.flox().build()?;

                match &self.command {
                    PackageCommands::Build(BuildArgs { installable }) => {
                        flox.package(installable.clone()).build().await?
                    }
                }

                Ok(())
            }
        }

        #[derive(Subcommand)]
        pub enum PackageCommands {
            Build(build::BuildArgs),
        }

        mod build {
            use clap::Args;
            use flox_rust_sdk::prelude::Installable;
            #[derive(Args)]
            pub struct BuildArgs {
                pub installable: Installable,
            }
        }
    }

    mod forward {
        use std::marker::PhantomData;

        use anyhow::Result;
        use clap::Args;

        use crate::run_in_flox;

        use super::FloxArgs;

        pub trait ForwardTo {
            const COMMAND: &'static [&'static str] = &["--help"];
        }

        #[derive(Args)]
        pub struct Forward<F: ForwardTo> {
            #[clap(raw = true)]
            args: Vec<String>,
            #[clap(skip)]
            _command_marker: PhantomData<F>,
        }
        impl<F: ForwardTo> Forward<F> {
            pub async fn run(&self, root: &FloxArgs) -> Result<()> {
                let mut command = Vec::new();
                if root.verbose {
                    command.append(&mut vec!["-v"]);
                }
                command.append(&mut F::COMMAND.to_vec());
                command.extend(self.args.iter().map(|s| &**s));
                command.extend(root.nix_args.iter().map(|s| &**s));
                run_in_flox(&command).await?;
                Ok(())
            }
        }
    }
}

#[derive(clap::Subcommand, Debug)]
pub(crate) enum InitializeAction {
    Init {
        #[clap(value_parser, help = "The package name you are trying to initialize")]
        package_name: String,
        #[clap(value_parser, help = "The builder you would like to use.")]
        builder: String,
    },
}

pub async fn run_in_flox(args: &[impl AsRef<std::ffi::OsStr> + Debug]) -> Result<ExitStatus> {
    debug!("Running in flox with arguments: {:?}", args);
    let status = Command::new(FLOX_SH)
        .args(args)
        .envs(&build_flox_env().unwrap())
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
