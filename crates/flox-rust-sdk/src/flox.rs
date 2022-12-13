use std::{
    collections::{BTreeSet, HashMap},
    fs::File,
    path::PathBuf,
};

use runix::{
    arguments::{
        common::NixCommonArgs,
        config::NixConfigArgs,
        flake::{FlakeArgs, OverrideInputs},
        EvalArgs, NixArgs,
    },
    command::Eval,
    command_line::{DefaultArgs, NixCommandLine},
    installable::Installable,
    NixBackend, RunJson,
};
use serde::Deserialize;
use thiserror::Error;

use crate::{
    actions::environment::Environment,
    actions::{environment::EnvironmentError, package::Package},
    environment::{self, build_flox_env},
    models::channels::ChannelRegistry,
    prelude::Stability,
    providers::git::GitProvider,
};

pub use crate::models::flox_installable::*;

lazy_static! {
    static ref INPUT_CHARS: Vec<char> = ('a'..='t').into_iter().collect();
}

pub const FLOX_SH: &str = env!("FLOX_SH");
pub const FLOX_VERSION: &str = env!("FLOX_VERSION");

/// The main API struct for our flox implementation
///
/// A [Flox] instance serves as the context for nix invocations
/// and possibly other tools such as git.
/// As a CLI application one invocation of `flox` would run on the same instance
/// but may call different methods.
///
/// [Flox] will provide a preconfigured instance of the Nix API.
/// By default this nix API uses the nix CLI.
/// Preconfiguration includes environment variables and flox specific arguments.
#[derive(Debug)]
pub struct Flox {
    /// The directory pointing to the users flox configuration
    ///
    /// TODO: set a default in the lib or CLI?
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub temp_dir: PathBuf,

    pub channels: ChannelRegistry,

    /// Whether to collect metrics of any kind
    /// (yet to be made use of)
    pub collect_metrics: bool,

    pub system: String,
}

pub trait FloxNixApi: NixBackend {
    fn new(flox: &Flox, default_nix_args: DefaultArgs) -> Self;
}

impl FloxNixApi for NixCommandLine {
    fn new(_: &Flox, default_nix_args: DefaultArgs) -> NixCommandLine {
        NixCommandLine {
            nix_bin: Some(environment::NIX_BIN.to_string()),
            defaults: default_nix_args,
        }
    }
}

/// Typed matching installable outputted by our Nix evaluation
#[derive(Deserialize, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct InstallableEvalQueryEntry {
    system: Option<String>,
    prefix: String,
    key: Vec<String>,
    input: String,
}

#[derive(Error, Debug)]
pub enum ResolveFloxInstallableError<Nix: FloxNixApi>
where
    Eval: RunJson<Nix>,
{
    #[error("No matches were found for the provided installable")]
    NoMatches,
    #[error("Error checking for installable matches: {0}")]
    Eval(<Eval as RunJson<Nix>>::JsonError),
    #[error("Error parsing installable eval output: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Typed output of our Nix evaluation to find matching installables
type InstallableEvalQueryOut = BTreeSet<InstallableEvalQueryEntry>;

#[derive(Debug)]
pub struct ResolvedInstallableMatch {
    pub prefix: String,
    pub system: Option<String>,
    pub key: Vec<String>,
    pub installable: Installable,
}

impl ResolvedInstallableMatch {
    fn new(
        flakeref: String,
        prefix: String,
        system: Option<String>,
        key: Vec<String>,
    ) -> ResolvedInstallableMatch {
        // Build the multi-part key into a Nix-safe single string
        let nix_str_key = key
            .iter()
            .map(|s| format!("{:?}", s))
            .collect::<Vec<_>>()
            .join(".");

        // Return our match as a single valid `Installable`
        ResolvedInstallableMatch {
            installable: Installable {
                flakeref,
                // Join the prefix and key into a safe attrpath, adding the associated system if present
                attr_path: match system {
                    Some(ref s) => format!("{:?}.{:?}.{}", &prefix, s, nix_str_key),
                    None => format!("{:?}.{}", &prefix, nix_str_key),
                },
            },
            prefix,
            system,
            key,
        }
    }
}

impl Flox {
    /// Provide the package scope to interact with raw packages, (build, develop, etc)
    pub fn package(
        &self,
        installable: Installable,
        stability: Stability,
        nix_arguments: Vec<String>,
    ) -> Package {
        Package::new(self, installable, stability, nix_arguments)
    }

    pub fn environment(&self, dir: PathBuf) -> Result<Environment, EnvironmentError> {
        Environment::new(self, dir)
    }

    /// Invoke Nix to convert a FloxInstallable into a list of matches
    pub async fn resolve_matches<Nix: FloxNixApi>(
        &self,
        flox_installable: FloxInstallable,
        default_flakerefs: &[&str],
        default_attr_prefixes: &[(&str, bool)],
    ) -> Result<Vec<ResolvedInstallableMatch>, ResolveFloxInstallableError<Nix>>
    where
        Eval: RunJson<Nix>,
    {
        assert!(default_flakerefs.len() <= INPUT_CHARS.len());

        // Optimize for installable resolutions that do not require an eval
        // Match against exactly 1 flakeref and 1 prefix
        if let ([d_flakeref], [(d_prefix, d_systemized)], [key]) = (
            default_flakerefs,
            default_attr_prefixes,
            flox_installable.attr_path.as_slice(),
        ) {
            return Ok(vec![ResolvedInstallableMatch::new(
                flox_installable
                    .source
                    .unwrap_or_else(|| d_flakeref.to_string()),
                d_prefix.to_string(),
                d_systemized.then(|| self.system.to_string()),
                vec![key.to_string()],
            )]);
        }

        // Create a map between input name and the input flakeref
        // such as `"a" => "github:NixOS/nixpkgs"`
        let mut flakeref_inputs: HashMap<String, String> = HashMap::new();

        // Add either the provided source flakeref or the default flakerefs to the inputs map
        if let Some(source) = flox_installable.source {
            flakeref_inputs.insert("a".to_string(), source);
        } else {
            for (flakeref, input) in default_flakerefs.iter().zip(INPUT_CHARS.iter()) {
                flakeref_inputs.insert(input.to_string(), flakeref.to_string());
            }
        }

        // Strip the systemization off of the default attr prefixes (only used in optimization)
        let mut attr_prefixes: Vec<&str> = default_attr_prefixes
            .iter()
            .map(|(prefix, _)| *prefix)
            .collect();

        // Split the key out of the provided attr path, using the first component as a prefix if more than 1 is present
        let key = match flox_installable.attr_path.split_first() {
            Some((prefix, key)) if key.len() > 0 => {
                attr_prefixes.push(prefix);
                Some(key.to_vec())
            }
            Some((prefix, _)) => Some(vec![prefix.clone()]),
            None => None,
        };

        // Construct the `apply` argument for the nix eval call to find what installables match
        let eval_apply = format!(
            // Template the Nix expression and our arguments in
            r#"(x: x {{
                system = "{system}";
                prefixes = [{prefixes}];
                inputs = [{inputs}];
                key = {key};
            }})"#,
            system = self.system,
            prefixes = attr_prefixes
                .into_iter()
                .map(|p| format!("{:?}", p))
                .collect::<Vec<_>>()
                .join(" "),
            inputs = flakeref_inputs
                .keys()
                .map(|k| format!("{:?}", k))
                .collect::<Vec<_>>()
                .join(" "),
            key = key
                .map(|x| format!(
                    "[{}]",
                    x.iter()
                        .map(|p| format!("{:?}", p))
                        .collect::<Vec<_>>()
                        .join(" ")
                ))
                .unwrap_or_else(|| "null".to_string()),
        );

        // The super resolver we're currently using to evaluate multiple whole flakerefs at once
        let resolve_installable: Installable =
            format!("path://{}#resolve", env!("FLOX_RESOLVER_SRC")).into();

        let command = Eval {
            flake: FlakeArgs {
                no_write_lock_file: true.into(),
                // Use the flakeref map from earlier as input overrides so all the inputs point to the correct flakerefs
                override_inputs: flakeref_inputs
                    .iter()
                    .map(|(c, flakeref)| OverrideInputs {
                        from: c.to_string(),
                        to: flakeref.to_string(),
                    })
                    .collect(),
            },
            // Use the super resolver as the installable (which we use as this only takes one)
            installable: resolve_installable.into(),
            eval_args: EvalArgs {
                apply: eval_apply.into(),
            },
            ..Default::default()
        };

        // Run our eval command with a typed output
        let json_out = command
            .run_json(&self.nix::<Nix>(vec![]), &NixArgs::default())
            .await
            .map_err(ResolveFloxInstallableError::Eval)?;
        let out: InstallableEvalQueryOut = serde_json::from_value(json_out)?;

        debug!("Output of installables eval query {:?}", out);

        // Map over the eval query output, including the inputs' flakerefs correlated from the flakeref mapping
        Ok(out
            .into_iter()
            .map(|e| {
                ResolvedInstallableMatch::new(
                    flakeref_inputs
                        .get(&e.input)
                        .expect("Match came from input that was not specified")
                        .to_string(),
                    e.prefix,
                    e.system,
                    e.key,
                )
            })
            .collect())
    }

    /// Produce a new Nix Backend
    ///
    /// This method performs backend independen configuration of nix
    /// and passes itself and the default config to the constructor of the Nix Backend
    ///
    /// The constructor will perform backend specifc configuration measures
    /// and return a fresh initialized backend.
    pub fn nix<Nix: FloxNixApi>(&self, extra_args: Vec<String>) -> Nix {
        // Set the registry file as default argument
        let registry_file = self.temp_dir.join("registry.json");
        serde_json::to_writer(File::create(&registry_file).unwrap(), &self.channels).unwrap();

        let config_args = NixConfigArgs {
            extra_experimental_features: ["nix-command", "flakes"]
                .map(String::from)
                .to_vec()
                .into(),

            extra_substituters: ["https://cache.floxdev.com?trusted=1"]
                .map(String::from)
                .to_vec()
                .into(),

            flake_registry: Some(registry_file.into()),

            ..Default::default()
        };

        let common_args = NixCommonArgs {
            ..Default::default()
        };

        let default_nix_args = DefaultArgs {
            environment: build_flox_env(),
            config_args,
            common_args,
            extra_args,
            ..Default::default()
        };

        Nix::new(self, default_nix_args)
    }

    /// Initialize and provide a git abstraction
    pub fn git_provider<Git: GitProvider>(&self) -> Git {
        Git::new()
    }
}
