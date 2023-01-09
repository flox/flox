use std::any::TypeId;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use bpaf::Parser;
use crossterm::tty::IsTty;
use flox_rust_sdk::flox::{Flox, FloxInstallable, ResolvedInstallableMatch};
use flox_rust_sdk::prelude::{Channel, ChannelRegistry, Installable};
use indoc::indoc;
use itertools::Itertools;
use log::{debug, error, warn};
use once_cell::sync::Lazy;

pub mod colors;
mod completion;
pub mod dialog;
pub mod init;
pub mod installables;
pub mod logger;
pub mod metrics;

use regex::Regex;

use self::completion::FloxCompletionExt;
use crate::utils::dialog::InquireExt;

static NIX_IDENTIFIER_SAFE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^[a-zA-Z0-9_-]+$"#).unwrap());

pub fn init_channels() -> Result<ChannelRegistry> {
    let mut channels = ChannelRegistry::default();
    channels.register_channel("flox", Channel::from_str("github:flox/floxpkgs")?);
    channels.register_channel("nixpkgs", Channel::from_str("github:flox/nixpkgs/stable")?);
    channels.register_channel(
        "nixpkgs-flox",
        Channel::from_str("github:flox/nixpkgs-flox/master")?,
    );

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

    Ok(channels)
}

fn nix_str_safe(s: &str) -> Cow<str> {
    if NIX_IDENTIFIER_SAFE.is_match(s) {
        s.into()
    } else {
        format!("{:?}", s).into()
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
/// runnables, i.e. packages (found in `packages` or `legacyPacakges`)
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

    /// Completion fucntion for bpaf completion engine
    fn complete_installable(&self) -> Vec<(String, Option<String>)> {
        #[allow(clippy::type_complexity)]
        static COMPLETED_INSTALLABLES: Lazy<
            Mutex<HashMap<(TypeId, String), Vec<(String, Option<String>)>>>,
        > = Lazy::new(|| Mutex::new(HashMap::new()));

        COMPLETED_INSTALLABLES
            .lock()
            .unwrap()
            .entry((TypeId::of::<Self>(), self.installable.clone()))
            .or_insert_with(|| {
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
            })
            .to_vec()
    }
}
impl<Matching: InstallableDef + 'static> InstallableArgument<Parsed, Matching> {
    async fn resolve_matches(&self, flox: &Flox) -> Result<Vec<ResolvedInstallableMatch>> {
        let drv = InstallableKind::any(Matching::DERIVATION_TYPES).unwrap();

        Ok(flox
            .resolve_matches(
                &[self.installable.clone()],
                &drv.flake_refs,
                &drv.prefix,
                false,
            )
            .await?)
    }

    /// called at runtime to extract single installable from CLI input
    pub async fn resolve_installable(&self, flox: &Flox) -> Result<Installable> {
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

    pub fn positional() -> impl Parser<Option<Self>> {
        let parser = bpaf::positional::<Unparsed>("INSTALLABLE");
        Self::parse_with(parser)
    }

    pub fn parse_with(parser: impl Parser<Unparsed>) -> impl Parser<Option<Self>> {
        let unparsed = parser
            .map(InstallableArgument::<Unparsed, Matching>::unparsed)
            .complete(|u| u.complete_installable());

        unparsed
            .map(|u| u.parse())
            .guard(Result::is_ok, "Is not ok")
            .map(Result::unwrap)
            .optional()
            .catch()
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
) -> Result<Installable> {
    match matches.len() {
        0 => {
            bail!("No matching installables found");
        },
        1 => Ok(matches.remove(0).installable()),
        _ => {
            let mut prefixes_with: HashMap<String, HashSet<String>> = HashMap::new();
            let mut flakerefs_with: HashMap<String, HashSet<String>> = HashMap::new();

            for m in &matches {
                let k1 = m.key.get(0).expect("match is missing key");

                flakerefs_with
                    .entry(k1.clone())
                    .or_insert_with(HashSet::new)
                    .insert(m.flakeref.clone());

                prefixes_with
                    .entry(k1.clone())
                    .or_insert_with(HashSet::new)
                    .insert(m.prefix.clone());
            }

            // Complile a list of choices for the user to choose from, and shorter choices for suggestions
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

                        let prefixes_total = prefixes_with.values().fold(0, |a, p| a + p.len());

                        let flakeref_str: Cow<str> = if flakerefs > 1 {
                            format!("{}#", m.flakeref).into()
                        } else {
                            "".into()
                        };

                        let prefix_strs: (Cow<str>, Cow<str>) = if prefixes_total > 1 {
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

                        (
                            format!("{}{}{}", flakeref_str, prefix_strs.0, nix_safe_key),
                            format!("{}{}{}", flakeref_str, prefix_strs.1, nix_safe_key),
                        )
                    },
                )
                .collect();

            let full_subcommand: Cow<str> = match arg_flag {
                Some(f) => format!("{subcommand} {f}").into(),
                None => subcommand.into(),
            };

            if !std::io::stderr().is_tty() || !std::io::stdin().is_tty() {
                error!(
                    indoc! {"
                    You must address a specific {derivation_type}. For example with:

                      $ flox {full_subcommand} {first_choice},

                    The available packages are:
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
            let sel = inquire::Select::new(
                &format!("Select a {} for flox {}", derivation_type, subcommand),
                choices.iter().map(|(long, _)| long).collect(),
            )
            .with_flox_theme()
            .raw_prompt()
            .with_context(|| format!("Failed to prompt for {} choice", derivation_type))?;

            let installable = matches.remove(sel.index).installable();

            warn!(
                "HINT: avoid selecting a {} next time with:\n  $ flox {} {}",
                derivation_type,
                full_subcommand,
                shell_escape::escape(choices.remove(sel.index).1.into())
            );

            Ok(installable)
        },
    }
}
