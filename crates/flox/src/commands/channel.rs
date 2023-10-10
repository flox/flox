use std::collections::{HashMap, HashSet};
use std::io::{BufWriter, Write};
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use bpaf::Bpaf;
use derive_more::Display;
use flox_rust_sdk::flox::{Flox, DEFAULT_OWNER};
use flox_rust_sdk::models::search::{
    do_search,
    Query,
    Registry,
    RegistryDefaults,
    RegistryInput,
    SearchParams,
    SearchResult,
    SearchResults,
    ShowError,
};
use flox_rust_sdk::nix::command::FlakeMetadata;
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::nix::flake_ref::git_service::{GitServiceAttributes, GitServiceRef};
use flox_rust_sdk::nix::flake_ref::FlakeRef;
use flox_rust_sdk::nix::RunJson;
use itertools::Itertools;
use regex::Regex;
use serde_json::json;

use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Select, Text};
use crate::utils::init::{DEFAULT_CHANNELS, HIDDEN_CHANNELS};

const SEARCH_INPUT_SEPARATOR: &'_ str = ":";
const SEARCH_INPUT_SEPARATOR: &'_ str = ":";

#[derive(Bpaf, Clone)]
pub struct ChannelArgs {}

#[derive(Debug, PartialEq, PartialOrd, Ord, Eq, Display, Clone, Copy)]
enum ChannelType {
    #[display(fmt = "user")]
    User,
    #[display(fmt = "flox")]
    Flox,
}

/// Search packages in subscribed channels
#[derive(Bpaf, Clone)]
pub struct Search {
    #[bpaf(short, long, argument("channel"))]
    pub channel: Vec<ChannelRef>,

    /// print search as JSON
    #[bpaf(long)]
    pub json: bool,

    /// print extended search results
    #[bpaf(short, long, long("verbose"), short('v'))]
    pub long: bool,

    /// force update of catalogs from remote sources before searching
    #[bpaf(long)]
    pub refresh: bool,

    /// query string of the form `<REGEX>[@<SEMVER-RANGE>]` used to filter
    /// match against package names/descriptions, and semantic version.
    /// Regex pattern is `PCRE` style, and semver ranges use the
    /// `node-semver` syntax.
    /// Exs: `(hello|coreutils)`, `node@>=16`, `coreutils@9.1`
    #[bpaf(positional("search-term"))]
    pub search_term: String,
}

// Try Using:
//   $ FLOX_FEATURES_CHANNELS=rust ./target/debug/flox search hello;
// Your first run will be slow, it's creating databases, but after that -
//   it's fast!
//
// `NIX_CONFIG='allow-import-from-derivation = true'` may be required because
// `pkgdb` disables this by default, but some flakes require it.
// Ideally this setting should be controlled by Registry preferences,
// which is TODO.
// Luckily most flakes don't.
impl Search {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("search");

        let search_params = construct_search_params(&self.search_term, &flox)?;

        let (results, exit_status) = do_search(&search_params)?;

        // Render what we have no matter what, then indicate whether we encountered an error.
        // FIXME: We may have warnings on `stderr` even with a successful call to `pkgdb`.
        //        We aren't checking that at all at the moment because better overall error handling
        //        is coming in a later PR.
        if self.json {
            render_search_results_json(results)?;
        } else {
            render_search_results_user_facing(results)?;
        }
        if exit_status.success() {
            Ok(())
        } else {
            bail!(
                "pkgdb exited with status code: {}",
                exit_status.code().unwrap_or(-1),
            );
        }
    }
}

fn construct_search_params(search_term: &str, flox: &Flox) -> Result<SearchParams> {
    // Create `registry` parameter for `pkgdb`
    let (inputs, priority) = collect_manifest_inputs(flox);
    let registry = Registry {
        inputs,
        priority,
        defaults: RegistryDefaults::default(),
    };

    // We've already checked that the search term is Some(_)
    let query = Query::from_str(search_term)?;

    Ok(SearchParams {
        registry,
        query,
        systems: Some(vec![flox.system.clone()]),
        ..SearchParams::default()
    })
}

/// This function is a hack to convert the current subscriptions into a format
/// that matches the search spec, which expects sources to come from the manifest.
///
/// This is temporary and will be removed once we have a functioning manifest.
fn collect_manifest_inputs(flox: &Flox) -> (HashMap<String, RegistryInput>, Vec<String>) {
    let channels = flox
        .channels
        .iter()
        .filter_map(|entry| {
            if HIDDEN_CHANNELS.contains_key(&*entry.from.id) {
                None
            } else if DEFAULT_CHANNELS.contains_key(&*entry.from.id) {
                Some((ChannelType::Flox, entry))
            } else {
                Some((ChannelType::User, entry))
            }
        })
        .sorted();

    // Create `registry` parameter for `pkgdb`
    let mut priority: Vec<String> = Vec::new();
    let mut inputs = HashMap::new();
    for (_, entry) in channels {
        priority.push(entry.from.id.to_string());
        let input = RegistryInput {
            from: entry.to.clone(),
            // TODO: handle `subtrees` and `stabilities`
            subtrees: None,
            stabilities: None,
        };
        inputs.insert(entry.from.id.to_string(), input);
    }
    (inputs, priority)
}

/// An intermediate representation of a search result used for rendering
#[derive(Debug, PartialEq, Clone)]
struct DisplayItem {
    /// The input that the package came from
    input: String,
    /// The displayable part of the package's attribute path
    package: String,
    /// The package description
    description: Option<String>,
    /// Whether to join the `input` and `package` fields with a separator when rendering
    render_with_input: bool,
}

fn render_search_results_user_facing(search_results: SearchResults) -> Result<()> {
    // Nothing to display
    if search_results.results.is_empty() {
        return Ok(());
    }
    // Search results contain a lot of information, but all we need for rendering are
    // the input, the package subpath (e.g. "python310Packages.flask"), and the description.
    let display_items = search_results
        .results
        .into_iter()
        .map(|r| {
            Ok(DisplayItem {
                input: r.input,
                package: r.pkg_subpath.join("."),
                description: r.description.map(|s| s.replace('\n', " ")),
                render_with_input: false,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let deduped_display_items = dedup_and_disambiguate_display_items(display_items);
    if deduped_display_items.is_empty() {
        bail!("deduplicating search results failed");
    }

    let column_width = deduped_display_items
        .iter()
        .map(|d| {
            if d.render_with_input {
                d.input.len() + d.package.len() + SEARCH_INPUT_SEPARATOR.len()
            } else {
                d.package.len()
            }
        })
        .max()
        .unwrap(); // SAFETY: could panic if `deduped_display_items` is empty, but we know it's not

    // Finally print something
    let mut writer = BufWriter::new(std::io::stdout());
    let default_desc = String::from("<no description provided>");
    for d in deduped_display_items.into_iter() {
        let package = if d.render_with_input {
            [d.input, d.package].join(SEARCH_INPUT_SEPARATOR)
        } else {
            d.package
        };
        let desc: String = d.description.unwrap_or(default_desc.clone());
        writeln!(&mut writer, "{package:<column_width$}  {desc}")?;
    }
    Ok(())
}

fn render_search_results_json(search_results: SearchResults) -> Result<()> {
    let json = serde_json::to_string(&search_results.results)?;
    println!("{}", json);
    Ok(())
}

/// Deduplicate and disambiguate display items.
///
/// This gets complicated because we have to satisfy a few constraints:
/// - The order of results from `pkgdb` is important (best matches come first),
///   so that order must be preserved.
/// - Versions shouldn't appear in the output, so multiple package versions from a single
///   input should be deduplicated.
/// - Packages that appear in more than one input need to be disambiguated by prepending
///   the name of the input and a separator.
fn dedup_and_disambiguate_display_items(mut display_items: Vec<DisplayItem>) -> Vec<DisplayItem> {
    let mut package_to_inputs: HashMap<String, HashSet<String>> = HashMap::new();
    for d in display_items.iter() {
        // Build a collection of packages and which inputs they are seen in so we can tell
        // which packages need to be disambiguated when rendering search results.
        package_to_inputs
            .entry(d.package.clone())
            .and_modify(|inputs| {
                inputs.insert(d.input.clone());
            })
            .or_insert_with(|| HashSet::from_iter([d.input.clone()]));
    }

    // For any package that comes from more than one input, mark it as needing to be joined
    for d in display_items.iter_mut() {
        if let Some(inputs) = package_to_inputs.get(&d.package) {
            d.render_with_input = inputs.len() > 1;
        }
    }

    // For each package in the search results, `package_to_inputs` contains the set of
    // inputs that the package is found in. Logically `package_to_inputs` contains
    // (package, input) pairs. If the `package` and `input` from a `DisplayItem` are
    // found in `package_to_inputs` it means that we have not yet seen this (package, input)
    // pair and we should render it (e.g. add it to `deduped_display_items`). Once we've
    // done that we remove this (package, input) pair from `package_to_inputs` so that
    // we never see that pair again.
    let mut deduped_display_items = Vec::new();
    for d in display_items.into_iter() {
        if let Some(inputs) = package_to_inputs.get_mut(d.package.as_str()) {
            // Remove this input so this (package, input) pair is never seen again
            if inputs.remove(&d.input) {
                deduped_display_items.push(d.clone());
            }
            if inputs.is_empty() {
                package_to_inputs.remove(&d.package);
            }
        }
    }

    deduped_display_items
}

/// Show detailed package information
#[derive(Bpaf, Clone)]
pub struct Show {
    /// The package to show detailed information about. Must be an exact match
    /// for a package name e.g. something copy-pasted from the output of `flox search`.
    #[bpaf(positional("search-term"))]
    pub search_term: String,
}

impl Show {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("show");
        let search_params = construct_show_params(&self.search_term, &flox)?;

        let (search_results, exit_status) = do_search(&search_params)?;

        if search_results.results.is_empty() {
            bail!("no packages matched this search term: {}", self.search_term);
        }
        // Render what we have no matter what, then indicate whether we encountered an error.
        // FIXME: We may have warnings on `stderr` even with a successful call to `pkgdb`.
        //        We aren't checking that at all at the moment because better overall error handling
        //        is coming in a later PR.
        render_show(search_results.results.as_slice())?;
        if exit_status.success() {
            Ok(())
        } else {
            bail!(
                "pkgdb exited with status code: {}",
                exit_status.code().unwrap_or(-1),
            );
        }
    }
}

fn construct_show_params(search_term: &str, flox: &Flox) -> Result<SearchParams> {
    let parts = search_term
        .split(SEARCH_INPUT_SEPARATOR)
        .map(String::from)
        .collect::<Vec<_>>();
    let (input_name, package_name) = match parts.as_slice() {
        [package_name] => (None, Some(package_name.to_owned())),
        [input_name, package_name] => (Some(input_name.to_owned()), Some(package_name.to_owned())),
        _ => Err(ShowError::InvalidSearchTerm(search_term.to_owned()))?,
    };

    // If we're given a specific input to search, only search that one,
    // otherwise build the whole list of inputs to search
    let (inputs, priority) = if let Some(input_name) = input_name {
        let Some(reg_input) = flox
            .channels
            .iter()
            .find(|entry| entry.from.id == input_name)
            .map(|entry| RegistryInput {
                from: entry.to.clone(),
                subtrees: None,
                stabilities: None,
            })
        else {
            bail!("manifest did not contain an input named '{}'", input_name)
        };
        let mut inputs = HashMap::new();
        inputs.insert(input_name.clone(), reg_input);
        (inputs, vec![input_name])
    } else {
        collect_manifest_inputs(flox)
    };

    // Only search the registry input that the search result comes from
    let registry = Registry {
        inputs,
        priority,
        ..Registry::default()
    };
    let query = Query {
        r#match: package_name,
        ..Query::default()
    };

    Ok(SearchParams {
        registry,
        query,
        ..SearchParams::default()
    })
}

fn render_show(search_results: &[SearchResult]) -> Result<()> {
    // FIXME: Proper rendering is coming later
    for package in search_results.iter() {
        let pkg_name = package.pkg_subpath.join(".");
        let description = package
            .description
            .as_ref()
            .map(|d| d.replace('\n', " "))
            .unwrap_or("<no description provided>".into());
        println!("{} - {}", pkg_name, description);
    }
    Ok(())
}

#[derive(Bpaf, Clone)]
pub enum SubscribeArgs {
    NameUrl {
        /// Name of the subscribed channel
        #[bpaf(positional("name"))]
        name: ChannelRef,
        /// Url of the channel.
        #[bpaf(positional("url"))]
        url: Url,
    },
    Name {
        /// Name of the subscribed channel
        #[bpaf(positional("name"))]
        name: ChannelRef,
    },
}

/// Subscribe to channel URL
#[derive(Bpaf, Clone)]
pub struct Subscribe {
    #[bpaf(external(subscribe_args), optional)]
    args: Option<SubscribeArgs>,
}
impl Subscribe {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("subscribe");
        // query name interactively if not provided
        let name = match &self.args {
            None => {
                Dialog {
                    help_message: None,
                    message: "Enter channel name to be added:",
                    typed: Text { default: None },
                }
                .prompt()
                .await?
            },
            Some(SubscribeArgs::Name { name }) | Some(SubscribeArgs::NameUrl { name, .. }) => {
                name.to_string()
            },
        };

        // return if name invalid
        if [HIDDEN_CHANNELS.keys(), DEFAULT_CHANNELS.keys()]
            .into_iter()
            .flatten()
            .contains(&name.as_str())
        {
            bail!("'{name}' is a reserved channel name");
        }

        // return if name is invalid
        if !Regex::new("^[a-zA-Z][a-zA-Z0-9_-]*$")
            .unwrap()
            .is_match(&name)
        {
            bail!("invalid channel name '{name}', valid regexp: ^[a-zA-Z][a-zA-Z0-9_-]*$");
        }

        // query url interactively if not provided
        let url = match self.args {
            None | Some(SubscribeArgs::Name { .. }) => {
                let default = FlakeRef::Github(GitServiceRef::new(
                    name.to_string(),
                    "floxpkgs".to_string(),
                    GitServiceAttributes {
                        reference: Some("master".to_string()),
                        ..Default::default()
                    },
                ));

                Dialog {
                    help_message: None,
                    message: &format!("Enter URL for '{name}' channel:"),
                    typed: Text {
                        default: Some(&default.to_string()),
                    },
                }
                .prompt()
                .await?
            },
            Some(SubscribeArgs::NameUrl { url, .. }) => url.to_string(),
        };

        // attempt parsing url as flakeref (validation)
        let url = url
            .parse::<FlakeRef>()
            .with_context(|| format!("'{url}' is not a valid url"))?;

        // read user channels
        let floxmeta = flox
            .floxmeta(DEFAULT_OWNER)
            .await
            .context("Could not get default floxmeta")?;

        let mut user_meta = floxmeta
            .user_meta()
            .await
            .context("Could not read user metadata")?;
        let user_meta_channels = user_meta.channels.get_or_insert(Default::default());

        // ensure channel does not yet exist
        if user_meta_channels.contains_key(&name) {
            bail!("A channel subscription '{name}' already exists");
        }

        // validate the existence of the flake behind `url`
        // candidate for a flakeref extension?
        let nix = flox.nix::<NixCommandLine>(Default::default());
        let command = FlakeMetadata {
            flake_ref: Some(url.clone().into()),
            ..Default::default()
        };
        let _ = command
            .run_json(&nix, &Default::default())
            .await
            .map_err(|_| anyhow::anyhow!("Could not verify channel URL: '{url}'"))?;

        user_meta_channels.insert(name.to_string(), url.to_string());

        // tansactionally update user meta file
        floxmeta
            .set_user_meta(&user_meta, &format!("Subscribed to {url} as '{name}'"))
            .await?;
        Ok(())
    }
}

/// Unsubscribe from a channel
#[derive(Bpaf, Clone)]
pub struct Unsubscribe {
    /// Channel name to unsubscribe.
    ///
    /// If omitted, flow will prompt for the name interactively
    #[bpaf(positional("channel"), optional)]
    channel: Option<ChannelRef>,
}

impl Unsubscribe {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("unsubscribe");
        let floxmeta = flox
            .floxmeta(DEFAULT_OWNER)
            .await
            .context("Could not get default floxmeta")?;

        let mut user_meta = floxmeta
            .user_meta()
            .await
            .context("Could not read user metadata")?;
        let user_meta_channels = user_meta.channels.get_or_insert(Default::default());

        let channel = match self.channel {
            Some(channel) => channel.to_owned(),
            None => {
                let dialog = Dialog {
                    help_message: None,
                    message: "Enter channel name to be added:",
                    typed: Select {
                        options: user_meta_channels.keys().cloned().collect_vec(),
                    },
                };

                dialog.prompt().await?
            },
        };

        if HIDDEN_CHANNELS
            .keys()
            .chain(DEFAULT_CHANNELS.keys())
            .contains(&channel.as_str())
        {
            bail!("'{channel}' is a reserved channel name and can't be unsubscribed from");
        }

        if user_meta_channels.remove(&channel).is_none() {
            bail!("No subscription found for '{channel}'");
        }

        floxmeta
            .set_user_meta(&user_meta, &format!("Unsubscribed from '{channel}'"))
            .await?;
        Ok(())
    }
}

/// List all subscribed channels
#[derive(Bpaf, Clone)]
pub struct Channels {
    /// print channels as JSON
    #[bpaf(long)]
    json: bool,
}

impl Channels {
    pub fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("channels");
        let channels = flox
            .channels
            .iter()
            .filter_map(|entry| {
                if HIDDEN_CHANNELS.contains_key(&*entry.from.id) {
                    None
                } else if DEFAULT_CHANNELS.contains_key(&*entry.from.id) {
                    Some((ChannelType::Flox, entry))
                } else {
                    Some((ChannelType::User, entry))
                }
            })
            .sorted_by(|a, b| Ord::cmp(a, b));

        if self.json {
            let mut map = serde_json::Map::new();
            for (channel, entry) in channels {
                map.insert(
                    entry.from.id.to_string(),
                    json!({
                        "type": channel.to_string(),
                        "url": entry.to.to_string()
                    }),
                );
            }

            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::Value::Object(map))?
            )
        } else {
            let width = channels
                .clone()
                .map(|(_, entry)| entry.from.id.len())
                .reduce(|acc, e| acc.max(e))
                .unwrap_or(8);

            println!("{ch:<width$}   TYPE   URL", ch = "CHANNEL");
            for (channel, entry) in channels {
                println!(
                    "{from:<width$} | {ty} | {url}",
                    from = entry.from.id,
                    ty = channel,
                    url = entry.to
                )
            }
        }
        Ok(())
    }
}

pub type ChannelRef = String;
pub type Url = String;
