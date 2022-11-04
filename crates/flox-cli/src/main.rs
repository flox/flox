use anyhow::Result;
use bpaf::OptionParser;
// use clap::{Parser, Args};
use flox_rust_sdk::environment::build_flox_env;
use log::{debug, error, info, warn};
use std::fmt::Debug;
use std::process::{exit, ExitStatus};
use tokio::process::Command;

mod build;
mod config;
mod utils;
pub static FLOX_SH: &str = env!("FLOX_SH");

mod commands {
    use anyhow::Result;
    use bpaf::Bpaf;
    use flox_rust_sdk::prelude::FloxBuilder;

    // use forward::{Forward, ForwardTo};

    use self::package::PackageArgs;

    #[derive(Bpaf)]
    #[bpaf(options)]
    pub struct FloxArgs {
        verbose: bool,

        debug: bool,

        #[bpaf(external(commands))]
        command: Commands,

        #[bpaf(positional)]
        nix_args: Vec<String>,
    }

    impl FloxArgs {
        pub async fn handle(&self) -> Result<()> {
            match self.command {
                // Commands::Support(ref f) => f.run(self).await?,
                // Commands::Build(ref f) => f.run(&self).await?,
                Commands::Package(ref package) => package.handle(self).await?,
            }
            Ok(())
        }

        fn flox(&self) -> FloxBuilder {
            FloxBuilder::default()
        }
    }

    /// Transparent separation of different categories of commands
    #[derive(Bpaf)]
    pub enum Commands {
        Package(#[bpaf(external(package::package_args))] PackageArgs),
    }

    mod package {
        use anyhow::Result;
        use bpaf::Bpaf;

        use self::build::BuildArgs;

        use super::FloxArgs;

        #[derive(Bpaf)]
        pub struct PackageArgs {
            #[bpaf(external(package_commands))]
            pub command: PackageCommands,
        }

        impl PackageArgs {
            pub async fn handle(&self, root_args: &FloxArgs) -> Result<()> {
                let flox = root_args.flox().build()?;

                match &self.command {
                    PackageCommands::Build(BuildArgs { installable }) => {
                        flox.package(installable.clone().into()).build().await?
                    }
                }

                Ok(())
            }
        }

        #[derive(Bpaf)]
        pub enum PackageCommands {
            #[bpaf(command)]
            Build(#[bpaf(external(build::build_args))] build::BuildArgs),
        }

        mod build {
            use bpaf::Bpaf;
            #[derive(Bpaf)]
            pub struct BuildArgs {
                #[bpaf(positional("INSTALLABLE"))]
                pub installable: String,
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    if let Ok(value) = env::var("FLOX_PREVIEW") {
        if let Ok(true) = bool::from_str(&value) {
            run_rust_flox().await?;
            exit(0);
        }
    }
    info!("`FLOX_PREVIEW` unset or not \"true\", falling back to legacy flox");
    run_in_flox(&env::args_os().collect::<Vec<_>>()[1..]).await?;
    Ok(())
}

async fn run_rust_flox() -> Result<()> {
    let args = commands::flox_args().run();
    args.handle().await?;
    Ok(())
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
