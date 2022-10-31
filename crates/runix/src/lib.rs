/// Rust abstraction over the nix command line
/// Candidate for a standalone library to build arbitrary Nix commands in a safe manner
use anyhow::Result;
use async_trait::async_trait;
use derive_builder::Builder;
use derive_more::From;

pub mod installable;
use installable::Installable;

/// Abstract nix interface
///
/// Runs a command as described as [NixArgs] by the `args` parameter.
/// Implementing methods for each "nix command" is pointless
/// as the implementation can be more cleanly abstracted to sets of possible configuration.
/// The sets are modeled after their implementation in Nix.
///
/// Future extensions of this trait may include running with text/json/rnix deserialization
#[async_trait]
pub trait NixApi {
    /// passthru nix
    async fn run(&self, args: NixArgs) -> Result<()>;
}

trait MergeArgs {
    /// Merge with another NixCommonArgs instance in-place
    /// Useful to override/extend previouly globally set variables
    fn merge(&mut self, other: &Self) -> Result<()>;
}

/// These arguments correspond to nix config settings as defined in `nix.conf` or overridden on the commandline
/// and refer to the options defined in
/// - All implementations of Setting<_> ([approximation](https://cs.github.com/?scopeName=All+repos&scope=&q=repo%3Anixos%2Fnix+%2FSetting%3C%5Cw%2B%3E%2F))
#[derive(Builder, Clone, Default)]
pub struct NixConfig {}

/// Nix arguments
/// should be a proper struct + de/serialization to and from [&str]
#[derive(Builder)]
#[builder(pattern = "owned")]
pub struct NixArgs {
    /// Common arguments to the nix command
    #[builder(default)]
    common: NixCommonArgs,

    /// Nix configuration (overrides nix.conf)
    #[builder(default)]
    config: NixConfig,

    /// Arguments to the nix subcommand
    /// These may contain flake/evaluation args if applicable
    // #[builder(setter(skip))]
    command: Box<dyn NixCommand + Send + Sync>,
}

/// These arguments do not depend on the nix subcommand issued
/// and refer to the options defined in
/// - (libmain/common-args.cc)[https://github.com/NixOS/nix/blob/a6239eb5700ebb85b47bb5f12366404448361f8d/src/libmain/common-args.cc#L7-L81]
/// - (nix/main.cc)[https://github.com/NixOS/nix/blob/b7e8a3bf4cbb2448db860f65ea13ef2c64b6883b/src/nix/main.cc#L66-L110]
#[derive(Builder, Clone, Default)]
pub struct NixCommonArgs {}

/// Flake related arguments
/// Corresponding to the arguments defined in
/// [libcmd/installables.cc](https://github.com/NixOS/nix/blob/84cc7ad77c6faf1cda8f8a10f7c12a939b61fe35/src/libcmd/installables.cc#L26-L126)
#[derive(Builder, Clone, Default)]
pub struct FlakeArgs {}

/// Evaluation related arguments
/// Corresponding to the arguments defined in
/// [libcmd/common-eval-args.cc](https://github.com/NixOS/nix/blob/a6239eb5700ebb85b47bb5f12366404448361f8d/src/libcmd/common-eval-args.cc#L14-L74)
#[derive(Builder, Clone, Default)]
pub struct EvaluationArgs {}

/// Installable argument for commands taking a single Installable
/// ([approximately](https://github.com/NixOS/nix/search?q=InstallablesCommand)
#[derive(From, Clone)]
pub struct InstallableArg(Installable);

/// Installable argument for commands taking multiple Installables
/// ([approximately](https://github.com/NixOS/nix/search?q=InstallablesCommand)
#[derive(From, Default, Clone)]
#[from(forward)]
pub struct InstallablesArgs(Vec<Installable>);

// /// An enumeration of all implemented commands and a fallback wildcard command to call arbitrary nix commands
// #[derive(Clone)]
// pub enum NixCommand {
//     Build(command::BuildArgs),
//     /// Command and arguments passed to nix
//     Wildcard(Vec<String>),
// }

pub trait NixCommand {
    fn subcommand(&self) -> Vec<String>;
    fn flake_args(&self) -> Option<FlakeArgs> {
        None
    }
    fn eval_args(&self) -> Option<EvaluationArgs> {
        None
    }
    fn installables(&self) -> Option<InstallablesArgs> {
        None
    }
}

// impl NixCommand {}

pub mod command {
    use derive_builder::Builder;

    use super::{EvaluationArgs, FlakeArgs, InstallablesArgs, NixCommand};

    #[derive(Builder, Default, Clone)]
    #[builder(default)]
    pub struct Build {
        flake: FlakeArgs,
        eval: EvaluationArgs,
        #[builder(setter(into))]
        installables: InstallablesArgs,
    }

    impl NixCommand for Build {
        fn subcommand(&self) -> Vec<String> {
            vec!["build".to_owned()]
        }

        fn flake_args(&self) -> Option<FlakeArgs> {
            Some(self.flake.clone())
        }

        fn eval_args(&self) -> Option<EvaluationArgs> {
            Some(self.eval.clone())
        }

        fn installables(&self) -> Option<InstallablesArgs> {
            Some(self.installables.clone())
        }
    }
}

pub mod command_line {
    use std::{collections::HashMap, process::Stdio};

    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use derive_builder::Builder;
    use derive_more::Constructor;
    use tokio::process::Command;

    use super::{
        EvaluationArgs, FlakeArgs, InstallablesArgs, NixApi, NixArgs, NixCommand, NixCommonArgs,
        NixConfig,
    };

    /// Nix Implementation based on the Nix Command Line
    #[derive(Constructor, Builder, Default, Clone)]
    pub struct NixCommandLine {
        nix_bin: Option<String>,

        /// Environment
        environment: HashMap<String, String>,
        common_args: NixCommonArgs,
        flake_args: FlakeArgs,
        eval_args: EvaluationArgs,
        config: NixConfig,
    }

    impl NixCommandLine {
        pub async fn run_in_nix(&self, args: &Vec<&str>) -> Result<String> {
            let output = Command::new(self.nix_bin.as_deref().unwrap_or_else(|| "nix"))
                .envs(&self.environment)
                .args(args)
                .output()
                .await?;

            let nix_response = std::str::from_utf8(&output.stdout)?;
            let nix_err_response = std::str::from_utf8(&output.stderr)?;

            if !nix_err_response.is_empty() {
                println!(
                    "Error in nix response, {}, {}",
                    nix_err_response,
                    nix_err_response.len()
                );
                Err(anyhow!(
                    "FXXXX: Error in nix response, {}, {}",
                    nix_err_response,
                    nix_err_response.len()
                ))
            } else {
                Ok(nix_response.to_string())
            }
        }
    }

    #[async_trait]
    impl NixApi for NixCommandLine {
        /// Construct and run a nix command
        /// Merge with default argumens as applicable
        async fn run(&self, args: NixArgs) -> Result<()> {
            let mut command = Command::new(self.nix_bin.as_deref().unwrap_or_else(|| "nix"));
            command
                .envs(&self.environment)
                .args(dbg!(self.config.args()))
                .args(dbg!(self.common_args.args()))
                .args(dbg!(args.args()))
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());

            command
                .as_std()
                .get_args()
                .map(|arg| arg.to_string_lossy().to_string())
                .collect::<Vec<String>>()
                .join(" ");

            let mut child = command.spawn()?;

            let _ = child.wait().await?;

            // let nix_response = std::str::from_utf8(&output.stdout)?;
            // let nix_err_response = std::str::from_utf8(&output.stderr)?;

            // if !nix_err_response.is_empty() {
            //     println!(
            //         "Error in nix response, {}, {}",
            //         nix_err_response,
            //         nix_err_response.len()
            //     );
            //     Err(anyhow!(
            //         "FXXXX: Error in nix response, {}, {}",
            //         nix_err_response,
            //         nix_err_response.len()
            //     ))
            // } else {
            //     dbg!(output);
            //     Ok(())
            // }
            Ok(())
        }
    }

    pub trait ToArgs {
        fn args(&self) -> Vec<String>;
    }

    impl ToArgs for NixConfig {
        fn args(&self) -> Vec<String> {
            vec![]
        }
    }
    impl ToArgs for NixArgs {
        fn args(&self) -> Vec<String> {
            let mut acc = vec![];
            acc.append(&mut self.config.args());
            acc.append(&mut self.common.args());
            acc.append(&mut (*self.command.as_ref()).args());
            acc
        }
    }

    impl ToArgs for NixCommonArgs {
        fn args(&self) -> Vec<String> {
            vec![]
        }
    }

    impl ToArgs for FlakeArgs {
        fn args(&self) -> Vec<String> {
            vec![]
        }
    }

    impl ToArgs for EvaluationArgs {
        fn args(&self) -> Vec<String> {
            vec![]
        }
    }

    impl ToArgs for InstallablesArgs {
        fn args(&self) -> Vec<String> {
            self.0.iter().map(|i| i.to_nix()).collect()
        }
    }

    impl ToArgs for dyn NixCommand + Send + Sync {
        fn args<'a>(&self) -> Vec<String> {
            let mut acc = Vec::new();
            acc.append(&mut self.flake_args().map_or(Vec::new(), |a| a.args()));
            acc.append(&mut self.eval_args().map_or(Vec::new(), |a| a.args()));
            acc.append(&mut self.subcommand());
            acc.append(&mut self.installables().map_or(Vec::new(), |a| a.args()));
            acc
            //  ++; self.eval_args() ++ self.installables()
        }
    }
}

pub use command_line as default;
