use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io::Stderr;
use std::marker::PhantomData;
use std::path::Path;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use bpaf::Parser;
use flox_rust_sdk::flox::{Flox, FloxInstallable, ResolvedInstallableMatch};
use flox_rust_sdk::models::legacy_environment_ref::EnvironmentRef;
use flox_rust_sdk::prelude::FlakeAttribute;
use flox_rust_sdk::providers::git::{GitCommandProvider, GitProvider};
use indoc::indoc;
use itertools::Itertools;
use log::{debug, error, warn};
use once_cell::sync::Lazy;

pub mod colors;
mod completion;
pub mod dialog;
pub mod display;
pub mod init;
pub mod installables;
pub mod logger;
pub mod metrics;

use regex::Regex;
use tokio::sync::Mutex;

use self::completion::FloxCompletionExt;
use crate::utils::dialog::{Dialog, Select};

static NIX_IDENTIFIER_SAFE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^[a-zA-Z0-9_-]+$"#).unwrap());
pub static TERMINAL_STDERR: Lazy<Mutex<Stderr>> = Lazy::new(|| Mutex::new(std::io::stderr()));

fn nix_str_safe(s: &str) -> Cow<str> {
    if NIX_IDENTIFIER_SAFE.is_match(s) {
        s.into()
    } else {
        format!("{s:?}").into()
    }
}

/// Low level Installable type
///
/// Describes a specific flake output abstraction by its name,
/// _default_ `prefix` and _default_ `flake_ref`.
///
/// Default **does not** refer to the nix commands defaults but
/// the default for this kind of Installable
///
/// Eg. "App" is an installable type that has the default prefix
/// `apps`.
/// It is targeted by the `run` command which also accepts other
/// runnables, i.e. packages (found in `packages` or `legacyPackages`)
///
/// [InstallableKind] allows to compose multiple of these *ables
/// into [InstallableArgument]s
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct InstallableKind {
    name: Cow<'static, str>,
    prefix: Cow<'static, [(&'static str, bool)]>,
    flake_refs: Cow<'static, [&'static str]>,
}

impl InstallableKind {
    pub const fn new(
        name: &'static str,
        prefix: &'static [(&'static str, bool)],
        flake_refs: &'static [&'static str],
    ) -> Self {
        Self {
            name: Cow::Borrowed(name),
            prefix: Cow::Borrowed(prefix),
            flake_refs: Cow::Borrowed(flake_refs),
        }
    }

    pub const fn package() -> Self {
        Self::new(
            "package",
            &[("packages", true), ("legacyPackages", true)],
            &["."],
        )
    }

    pub const fn template() -> Self {
        Self::new("template", &[("templates", false)], &["flake:flox"])
    }

    pub const fn shell() -> Self {
        Self::new("shell", &[("devShells", true)], &["."])
    }

    pub const fn app() -> Self {
        Self::new("app", &[("apps", true)], &["."])
    }

    pub const fn bundler() -> Self {
        Self::new("bundler", &[("bundlers", true)], &[
            "github:flox/bundlers/master",
        ])
    }

    pub fn or(self, other: Self) -> Self {
        let name = self.name + other.name;

        // TODO: Needs sorted? If so, go though set?
        // this impl retains order.
        let mut prefix: Vec<(&str, bool)> = self.prefix.to_vec();
        prefix.extend(other.prefix.iter());
        prefix.dedup();

        let mut flake_refs: Vec<&str> = self.flake_refs.to_vec();
        flake_refs.extend(other.flake_refs.iter());
        flake_refs.dedup();

        Self {
            name,
            prefix: Cow::Owned(prefix.into_iter().collect::<Vec<_>>()),
            flake_refs: Cow::Owned(flake_refs.into_iter().collect::<Vec<_>>()),
        }
    }

    pub fn any<'a>(drv_types: impl IntoIterator<Item = &'a Self>) -> Option<Self> {
        drv_types.into_iter().dedup().cloned().reduce(Self::or)
    }
}

pub type Unparsed = String;
pub type Parsed = FloxInstallable;

///
#[derive(Clone, Debug, Default)]
pub struct InstallableArgument<InstallableState, Matching: InstallableDef> {
    installable: InstallableState,
    _matching: PhantomData<Matching>,
}

impl<Matching: InstallableDef + 'static> InstallableArgument<Unparsed, Matching> {
    pub fn unparsed(unparsed: String) -> Self {
        Self {
            installable: unparsed,
            _matching: Default::default(),
        }
    }

    /// Try to convert an [Unparsed] [InstallableArgument] to a [Parsed] one
    /// by parsing its inner value as [FloxInstallable]
    pub fn parse(self) -> Result<InstallableArgument<Parsed, Matching>> {
        Ok(InstallableArgument {
            installable: self.installable.parse()?,
            _matching: self._matching,
        })
    }

    /// Completion function for bpaf completion engine
    fn complete_installable(&self) -> Vec<(String, Option<String>)> {
        // avoid stray logs of lower severity from polluting the completions
        log::set_max_level(log::LevelFilter::Error);

        let drv = InstallableKind::any(Matching::DERIVATION_TYPES).unwrap();

        let installable = self.installable.clone();
        let default_prefixes = drv.prefix;
        let default_flakerefs = drv.flake_refs;

        let flox = Flox::completion_instance().expect("Could not initialize flox instance");

        let handle = tokio::runtime::Handle::current();
        let comp = std::thread::spawn(move || {
            handle
                .block_on(flox.complete_installable(
                    &installable,
                    &default_flakerefs,
                    &default_prefixes,
                ))
                .map_err(|e| debug!("Failed to complete installable: {e}"))
                .unwrap_or_default()
        })
        .join()
        .unwrap();

        comp.into_iter().map(|a| (a, None)).collect()
    }
}
impl<Matching: InstallableDef + 'static> InstallableArgument<Parsed, Matching> {
    async fn resolve_matches(&self, flox: &Flox) -> Result<Vec<ResolvedInstallableMatch>> {
        let drv = InstallableKind::any(Matching::DERIVATION_TYPES).unwrap();

        let matches = flox
            .resolve_matches::<_, GitCommandProvider>(
                &[self.installable.clone()],
                &drv.flake_refs,
                &drv.prefix,
                false,
                Matching::PROCESSOR,
            )
            .await?;
        Ok(matches)
    }

    /// called at runtime to extract single installable from CLI input
    pub async fn resolve_flake_attribute(&self, flox: &Flox) -> Result<FlakeAttribute> {
        let drv = InstallableKind::any(Matching::DERIVATION_TYPES).unwrap();
        let matches = self.resolve_matches(flox).await?;

        resolve_installable_from_matches(
            Matching::SUBCOMMAND,
            &drv.name,
            Matching::ARG_FLAG,
            matches,
        )
        .await
    }

    pub fn positional() -> impl Parser<Self> {
        let parser = bpaf::positional::<Unparsed>("INSTALLABLE");
        Self::parse_with(parser)
    }

    pub fn parse_with(parser: impl Parser<Unparsed>) -> impl Parser<Self> {
        let unparsed = parser
            .map(InstallableArgument::<Unparsed, Matching>::unparsed)
            .complete(|u| u.complete_installable());

        unparsed
            .map(|u| u.parse())
            .guard(Result::is_ok, "Is not ok")
            .map(Result::unwrap)
    }
}
impl<Matching: InstallableDef> FromStr for InstallableArgument<Unparsed, Matching> {
    type Err = <Unparsed as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            installable: Unparsed::from_str(s)?,
            _matching: Default::default(),
        })
    }
}
impl<Matching: InstallableDef> FromStr for InstallableArgument<Parsed, Matching> {
    type Err = <Parsed as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            installable: Parsed::from_str(s)?,
            _matching: Default::default(),
        })
    }
}

pub trait InstallableDef: Default + Clone {
    const DERIVATION_TYPES: &'static [InstallableKind];
    const SUBCOMMAND: &'static str;
    const ARG_FLAG: Option<&'static str> = None;
    const PROCESSOR: Option<&'static str>;
}

/// Resolve a single installation candidate from a list of matches
///
/// - return an error if no matches were found
/// - return a nix installable if a single match was found
/// - start an interactive dialog if multiple matches were found
///   and a controlling tty was detected
pub async fn resolve_installable_from_matches(
    subcommand: &str,
    derivation_type: &str,
    arg_flag: Option<&str>,
    mut matches: Vec<ResolvedInstallableMatch>,
) -> Result<FlakeAttribute> {
    match matches.len() {
        0 => {
            bail!("No matching installables found");
        },
        1 => Ok(matches.remove(0).flake_attribute()),
        _ => {
            let mut prefixes_total: HashSet<String> = HashSet::new();
            let mut prefixes_with: HashMap<String, HashSet<String>> = HashMap::new();
            let mut flakerefs_with: HashMap<String, HashSet<String>> = HashMap::new();

            for m in &matches {
                let k1 = m.key.get(0).expect("match is missing key");

                prefixes_total.insert(m.prefix.clone());

                flakerefs_with
                    .entry(k1.clone())
                    .or_insert_with(HashSet::new)
                    .insert(m.flakeref.clone());

                prefixes_with
                    .entry(k1.clone())
                    .or_insert_with(HashSet::new)
                    .insert(m.prefix.clone());
            }

            // Compile a list of choices for the user to choose from, and shorter choices for suggestions
            let mut choices: Vec<(String, String)> = matches
                .iter()
                .map(
                    // Format the results according to how verbose we have to be for disambiguation, only showing the flakeref or prefix when multiple are used
                    |m| {
                        let nix_safe_key = m
                            .key
                            .iter()
                            .map(|s| nix_str_safe(s.as_str()))
                            .collect::<Vec<_>>()
                            .join(".");

                        let k1 = m.key.get(0).expect("match is missing key");

                        let flakerefs = flakerefs_with.get(k1).map(HashSet::len).unwrap_or(0);
                        let prefixes = flakerefs_with.get(k1).map(HashSet::len).unwrap_or(0);

                        let flakeref_str: Cow<str> = if flakerefs > 1 {
                            format!("{}#", m.flakeref).into()
                        } else {
                            "".into()
                        };

                        let prefix_strs: (Cow<str>, Cow<str>) = if prefixes_total.len() > 1 {
                            let long: Cow<str> = format!("{}.", nix_str_safe(&m.prefix)).into();

                            let short = if prefixes > 1 {
                                long.clone()
                            } else {
                                "".into()
                            };

                            (long, short)
                        } else {
                            ("".into(), "".into())
                        };

                        let description_part: Cow<str> = match &m.description {
                            Some(d) => format!(": {d}").into(),
                            None => "".into(),
                        };

                        (
                            format!(
                                "{}{}{}{}",
                                flakeref_str, prefix_strs.0, nix_safe_key, description_part
                            ),
                            format!("{}{}{}", flakeref_str, prefix_strs.1, nix_safe_key),
                        )
                    },
                )
                .collect();

            let full_subcommand: Cow<str> = match arg_flag {
                Some(f) => format!("{subcommand} {f}").into(),
                None => subcommand.into(),
            };

            if !Dialog::can_prompt() {
                error!(
                    indoc! {"
                    You must address a specific {derivation_type}. For example with:

                      $ flox {full_subcommand} {first_choice},

                    The available {derivation_type}s are:
                    {choices_list}
                "},
                    derivation_type = derivation_type,
                    full_subcommand = full_subcommand,
                    first_choice = choices.get(0).expect("Expected at least one choice").1,
                    choices_list = choices
                        .iter()
                        .map(|(choice, _)| format!("  - {choice}"))
                        .join("\n")
                );

                bail!("No terminal to prompt for {derivation_type} choice");
            }

            // Prompt for the user to select match
            let dialog = Dialog {
                message: &format!("Select a {derivation_type} for flox {subcommand}"),
                help_message: None,
                typed: Select {
                    options: choices.iter().cloned().map(|(long, _)| long).collect(),
                },
            };

            let sel = dialog
                .raw_prompt()
                .await
                .with_context(|| format!("Failed to prompt for {derivation_type} choice"))?
                .0;

            let flake_attribute = matches.remove(sel).flake_attribute();

            warn!(
                "HINT: avoid selecting a {} next time with:\n  $ flox {} {}",
                derivation_type,
                full_subcommand,
                shell_escape::escape(choices.remove(sel).1.into())
            );

            Ok(flake_attribute)
        },
    }
}

/// Resolve a single environment from a list of matches
///
/// - return an error if no matches were found
/// - return the match if there is only one
/// - start an interactive dialog if multiple matches were found
///   and a controlling tty was detected
pub async fn resolve_environment_ref<'flox, Git: GitProvider + 'static>(
    flox: &'flox Flox,
    subcommand: &str,
    environment_name: Option<&str>,
) -> Result<EnvironmentRef<'flox>> {
    let mut environment_refs = EnvironmentRef::find::<_, Git>(flox, environment_name).await?;
    match environment_refs.len() {
        0 => {
            bail!("No matching environments found");
        },
        1 => Ok(environment_refs.remove(0)),
        _ => {
            let mut sources: HashSet<Option<&Path>> = HashSet::new();

            for m in &environment_refs {
                if let EnvironmentRef::Project(p) = m {
                    sources.insert(Some(&p.workdir));
                } else {
                    sources.insert(None);
                }
            }

            let current_dir = std::env::current_dir()?;

            // Compile a list of choices for the user to choose from, and shorter choices for suggestions
            let mut choices: Vec<(String, &String)> = environment_refs
                .iter()
                .map(
                    // Format the results according to how verbose we have to be for disambiguation, only showing the flakeref or prefix when multiple are used
                    |m| {
                        let prefix: Cow<str> = match m {
                            EnvironmentRef::Named(_) if sources.len() > 1 => "Named - ".into(),
                            EnvironmentRef::Project(n) if sources.len() > 1 => {
                                let rel = pathdiff::diff_paths(&n.workdir, &current_dir)
                                    .ok_or_else(|| anyhow!("Project path should be absolute"))?;

                                if rel == Path::new("") {
                                    ". - ".into()
                                } else {
                                    format!("{} - ", rel.display()).into()
                                }
                            },
                            _ => "".into(),
                        };

                        let name = match m {
                            EnvironmentRef::Named(n) => &n.name,
                            EnvironmentRef::Project(p) => &p.name,
                        };

                        Ok((format!("{prefix}{name}"), name))
                    },
                )
                .collect::<Result<Vec<_>>>()?;

            if !Dialog::can_prompt() {
                error!(
                    indoc! {"
                    You must address a specific environment. For example with:

                      $ flox {subcommand} {first_choice},

                    The available environments are:
                    {choices_list}
                "},
                    subcommand = subcommand,
                    first_choice = choices.get(0).expect("Expected at least one choice").1,
                    choices_list = choices
                        .iter()
                        .map(|(long, _)| format!("  - {long}"))
                        .join("\n")
                );

                bail!("No terminal to prompt for environment choice");
            }

            // Prompt for the user to select match
            let dialog = Dialog {
                message: &format!("Select an environment for flox {subcommand}"),
                help_message: None,
                typed: Select {
                    options: choices.iter().cloned().map(|(long, _)| long).collect(),
                },
            };

            let (sel, _) = dialog
                .raw_prompt()
                .await
                .context("Failed to prompt for environment choice")?;

            let escaped = shell_escape::escape(choices.remove(sel).1.into()).into_owned();

            let environment_ref = environment_refs.remove(sel);

            warn!(
                "HINT: avoid selecting an environment next time with:\n  $ flox {subcommand} -e {escaped}",
            );

            Ok(environment_ref)
        },
    }
}
