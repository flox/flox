use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::PathBuf;

use derive_more::Constructor;
use log::{debug, info};
use once_cell::sync::Lazy;
use runix::arguments::common::NixCommonArgs;
use runix::arguments::config::NixConfigArgs;
use runix::arguments::flake::{FlakeArgs, OverrideInput};
use runix::arguments::{EvalArgs, NixArgs};
use runix::command::Eval;
use runix::command_line::{DefaultArgs, NixCommandLine};
use runix::installable::Installable;
use runix::{NixBackend, RunJson};
use serde::Deserialize;
use thiserror::Error;

use crate::actions::environment::{Environment, EnvironmentError};
use crate::actions::package::Package;
use crate::actions::project::{self, Project};
use crate::environment::{self, default_nix_subprocess_env};
use crate::models::channels::ChannelRegistry;
pub use crate::models::environment_ref::*;
pub use crate::models::flox_installable::*;
use crate::models::stability::Stability;
use crate::providers::git::GitProvider;

static INPUT_CHARS: Lazy<Vec<char>> = Lazy::new(|| ('a'..='t').into_iter().collect());

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

    /// access tokens injected in nix.conf
    ///
    /// Use [Vec] to preserve orginal ordering
    pub access_tokens: Vec<(String, String)>,
    pub netrc_file: PathBuf,

    pub channels: ChannelRegistry,

    pub system: String,

    pub uuid: uuid::Uuid,
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
#[serde(rename_all = "camelCase")]
struct InstallableEvalQueryEntry {
    system: Option<String>,
    explicit_system: bool,
    prefix: String,
    key: Vec<String>,
    input: String,
    description: Option<String>,
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

#[derive(Debug, Constructor)]
pub struct ResolvedInstallableMatch {
    pub flakeref: String,
    pub prefix: String,
    pub system: Option<String>,
    pub explicit_system: bool,
    pub key: Vec<String>,
    pub description: Option<String>,
}

impl ResolvedInstallableMatch {
    pub fn installable(self) -> Installable {
        // Build the multi-part key into a Nix-safe single string
        let nix_str_key = self
            .key
            .into_iter()
            .map(|s| format!("{:?}", s))
            .collect::<Vec<_>>()
            .join(".");

        Installable {
            flakeref: self.flakeref,
            // Join the prefix and key into a safe attrpath, adding the associated system if present
            attr_path: match self.system {
                Some(ref s) => format!(".{:?}.{:?}.{}", &self.prefix, s, nix_str_key),
                None => format!(".{:?}.{}", &self.prefix, nix_str_key),
            },
        }
    }
}

impl Flox {
    /// Provide the package scope to interact with raw packages, (build, develop, etc)
    ///
    ///  TODO: consume [Option<FloxInstallable>]
    pub fn package(
        &self,
        installable: Installable,
        stability: Stability,
        nix_arguments: Vec<String>,
    ) -> Package {
        Package::new(self, installable, stability, nix_arguments)
    }

    pub fn project<X>(&self, x: X) -> Project<project::Closed<X>> {
        Project::closed(self, x)
    }

    pub async fn environment_ref<Git: GitProvider>(
        &self,
        name: &str,
    ) -> Result<EnvironmentRef, EnvironmentRefError<Git::DiscoverError>> {
        EnvironmentRef::new::<Git>(self, name).await
    }

    pub fn environment(&self, dir: PathBuf) -> Result<Environment, EnvironmentError> {
        Environment::new(self, dir)
    }

    /// Invoke Nix to convert a FloxInstallable into a list of matches
    pub async fn resolve_matches<Nix: FloxNixApi>(
        &self,
        flox_installables: &[FloxInstallable],
        default_flakerefs: &[&str],
        default_attr_prefixes: &[(&str, bool)],
        must_exist: bool,
        processor: Option<&str>,
    ) -> Result<Vec<ResolvedInstallableMatch>, ResolveFloxInstallableError<Nix>>
    where
        Eval: RunJson<Nix>,
    {
        // Optimize for installable resolutions that do not require an eval
        // Match against exactly 1 flakeref and 1 prefix
        let mut optimized = vec![];
        for flox_installable in flox_installables {
            if let (false, [d_flakeref], [(d_prefix, d_systemized)], [key]) = (
                must_exist,
                default_flakerefs,
                default_attr_prefixes,
                flox_installable.attr_path.as_slice(),
            ) {
                optimized.push(ResolvedInstallableMatch::new(
                    flox_installable
                        .source
                        .as_ref()
                        .map(String::from)
                        .unwrap_or_else(|| d_flakeref.to_string()),
                    d_prefix.to_string(),
                    d_systemized.then(|| self.system.to_string()),
                    false,
                    vec![key.to_string()],
                    None,
                ));
            } else {
                break;
            }
        }
        if optimized.len() == flox_installables.len() {
            return Ok(optimized);
        }

        let numbered_flox_installables: Vec<(usize, FloxInstallable)> = flox_installables
            .iter()
            .enumerate()
            .map(|(i, f)| (i, f.clone()))
            .collect();

        let mut flakeref_inputs: HashMap<char, String> = HashMap::new();
        let mut inputs_assoc: HashMap<Option<usize>, Vec<char>> = HashMap::new();

        let has_sourceless = numbered_flox_installables
            .iter()
            .any(|(_, f)| f.source.is_none());

        let mut occupied = 0;

        if has_sourceless {
            for flakeref in default_flakerefs {
                flakeref_inputs.insert(*INPUT_CHARS.get(occupied).unwrap(), flakeref.to_string());
                inputs_assoc
                    .entry(None)
                    .or_insert_with(Vec::new)
                    .push(*INPUT_CHARS.get(occupied).unwrap());
                occupied += 1;
            }
        }

        for (installable_id, flox_installable) in &numbered_flox_installables {
            if let Some(ref source) = flox_installable.source {
                let assoc = inputs_assoc
                    .entry(Some(*installable_id))
                    .or_insert_with(Vec::new);

                if let Some((c, _)) = flakeref_inputs.iter().find(|(_, s)| *s == source) {
                    let c = *c;
                    flakeref_inputs.insert(c, source.to_string());
                    assoc.push(c);
                } else {
                    flakeref_inputs.insert(*INPUT_CHARS.get(occupied).unwrap(), source.to_string());
                    assoc.push(*INPUT_CHARS.get(occupied).unwrap());
                    occupied += 1;
                }
            }
        }

        // Strip the systemization off of the default attr prefixes (only used in optimization)
        let default_attr_prefixes: Vec<&str> = default_attr_prefixes
            .iter()
            .map(|(prefix, _)| *prefix)
            .collect();

        let installable_resolve_strs: Vec<String> = numbered_flox_installables
            .into_iter()
            .map(|(installable_id, flox_installable)| {
                // Split the key out of the provided attr path, using the first component as a prefix if more than 1 is present
                let (attr_prefix, key) = match flox_installable.attr_path.split_first() {
                    Some((prefix, key)) if !key.is_empty() => {
                        (Some(prefix.as_str()), Some(key.to_vec()))
                    },
                    Some((prefix, _)) => (None, Some(vec![prefix.clone()])),
                    None => (None, None),
                };

                format!(
                    // Template the Nix expression and our arguments in
                    r#"(x {{
                        system = "{system}";
                        defaultPrefixes = [{default_prefixes}];
                        prefix = {prefix};
                        inputs = [{inputs}];
                        key = {key};
                        processor = {processor};
                    }})"#,
                    system = self.system,
                    prefix = attr_prefix
                        .map(|p| format!("{:?}", p))
                        .unwrap_or_else(|| "null".to_string()),
                    default_prefixes = default_attr_prefixes
                        .iter()
                        .map(|p| format!("{:?}", p))
                        .collect::<Vec<_>>()
                        .join(" "),
                    inputs = inputs_assoc
                        .get(&None)
                        .iter()
                        .chain(inputs_assoc.get(&Some(installable_id)).iter())
                        .flat_map(|x| x
                            .iter()
                            .map(|x| format!("{:?}", x.to_string()))
                            .collect::<Vec<String>>())
                        .collect::<Vec<String>>()
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
                    processor = processor
                        .map(|x| format!("(prefix: key: item: {})", x))
                        .unwrap_or_else(|| "null".to_string()),
                )
                .replace("                    ", " ")
                .replace('\n', "")
            })
            .collect();

        // Construct the `apply` argument for the nix eval call to find what installables match
        let eval_apply = format!(r#"(x: ({}))"#, installable_resolve_strs.join(" ++ "));

        // The super resolver we're currently using to evaluate multiple whole flakerefs at once
        let resolve_installable: Installable =
            format!("path://{}#resolve", env!("FLOX_RESOLVER_SRC")).into();

        let command = Eval {
            flake: FlakeArgs {
                no_write_lock_file: true.into(),
                // Use the flakeref map from earlier as input overrides so all the inputs point to the correct flakerefs
                override_inputs: flakeref_inputs
                    .iter()
                    .map(|(c, flakeref)| OverrideInput {
                        from: c.to_string(),
                        to: flakeref.to_string(),
                    })
                    .collect(),
            },
            // Use the super resolver as the installable (which we use as this only takes one)
            eval_args: EvalArgs {
                installable: Some(resolve_installable.into()),
                apply: Some(eval_apply.into()),
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
                        .get(&e.input.chars().next().unwrap())
                        .expect("Match came from input that was not specified")
                        .to_string(),
                    e.prefix,
                    e.system,
                    e.explicit_system,
                    e.key,
                    e.description,
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
        use std::io::Write;
        use std::os::unix::prelude::OpenOptionsExt;

        let environment = {
            // Write registry file if it does not exist or has changed
            let global_registry_file = self.config_dir.join("floxFlakeRegistry.json");
            let registry_content = serde_json::to_string_pretty(&self.channels).unwrap();
            if !global_registry_file.exists() || {
                let contents: ChannelRegistry =
                    serde_json::from_reader(std::fs::File::open(&global_registry_file).unwrap())
                        .expect("Invalid registry file");

                contents != self.channels
            } {
                let temp_registry_file = self.temp_dir.join("registry.json");

                std::fs::File::options()
                    .mode(0o600)
                    .create_new(true)
                    .write(true)
                    .open(&temp_registry_file)
                    .unwrap()
                    .write_all(registry_content.as_bytes())
                    .unwrap();

                debug!("Updating flake registry: {global_registry_file:?}");
                std::fs::rename(temp_registry_file, &global_registry_file).unwrap();
            }

            let config = NixConfigArgs {
                accept_flake_config: true.into(),
                warn_dirty: false.into(),
                extra_experimental_features: ["nix-command", "flakes"]
                    .map(String::from)
                    .to_vec()
                    .into(),
                extra_substituters: ["https://cache.floxdev.com"]
                    .map(String::from)
                    .to_vec()
                    .into(),
                extra_trusted_public_keys: [
                    "flox-store-public-0:8c/B+kjIaQ+BloCmNkRUKwaVPFWkriSAd0JJvuDu4F0=",
                ]
                .map(String::from)
                .to_vec()
                .into(),
                extra_access_tokens: self.access_tokens.clone().into(),
                flake_registry: Some(global_registry_file.into()),
                netrc_file: Some(self.netrc_file.clone().into()),
                connect_timeout: 5.into(),
                ..Default::default()
            };

            let nix_config = format!(
                "# Automatically generated - do not edit.\n{}\n",
                config.to_config_string()
            );

            // Write nix.conf file if it does not exist or has changed
            let global_config_file_path = self.config_dir.join("nix.conf");
            if !global_config_file_path.exists() || {
                let mut contents = String::new();
                std::fs::File::open(&global_config_file_path)
                    .unwrap()
                    .read_to_string(&mut contents)
                    .unwrap();

                contents != nix_config
            } {
                let temp_config_file_path = self.temp_dir.join("nix.conf");

                std::fs::File::options()
                    .mode(0o600)
                    .create_new(true)
                    .write(true)
                    .open(&temp_config_file_path)
                    .unwrap()
                    .write_all(nix_config.as_bytes())
                    .unwrap();

                info!("Updating nix.conf: {global_config_file_path:?}");
                std::fs::rename(temp_config_file_path, &global_config_file_path).unwrap()
            }

            let mut env = default_nix_subprocess_env();
            let _ = env.insert(
                "NIX_USER_CONF_FILES".to_string(),
                global_config_file_path.to_string_lossy().to_string(),
            );
            env
        };

        #[allow(clippy::needless_update)]
        let common_args = NixCommonArgs {
            ..Default::default()
        };

        let default_nix_args = DefaultArgs {
            environment,
            common_args,
            extra_args,
            ..Default::default()
        };

        Nix::new(self, default_nix_args)
    }
}
