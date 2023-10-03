use std::collections::HashMap;
use std::env;
use std::io::{BufWriter, Write};
use std::process::Command;
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use bpaf::Bpaf;
use derive_more::Display;
use flox_rust_sdk::flox::{Flox, DEFAULT_OWNER};
use flox_rust_sdk::models::search::{
    Query,
    Registry,
    RegistryDefaults,
    RegistryInput,
    SearchParams,
    SearchResults,
};
use flox_rust_sdk::nix::command::FlakeMetadata;
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::nix::flake_ref::git_service::{GitServiceAttributes, GitServiceRef};
use flox_rust_sdk::nix::flake_ref::FlakeRef;
use flox_rust_sdk::nix::RunJson;
use itertools::Itertools;
use joinery::Joinable;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::json;

use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Select, Text};
use crate::utils::init::{DEFAULT_CHANNELS, HIDDEN_CHANNELS};

#[derive(Bpaf, Clone)]
pub struct ChannelArgs {}

#[derive(Debug, PartialEq, PartialOrd, Ord, Eq, Display, Clone, Copy)]
enum ChannelType {
    #[display(fmt = "user")]
    User,
    #[display(fmt = "flox")]
    Flox,
}

// This is the `PKGDB` path that we actually use.
// This is set once and prefers the `PKGDB` env variable, but will use
// the fallback to the binary available at build time if it is unset.
pub static PKGDB_BIN: Lazy<String> =
    Lazy::new(|| env::var("PKGDB").unwrap_or(env!("PKGDB_BIN").to_string()));

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
    pub search_term: Option<String>,
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

        if self.search_term.is_none() {
            bail!(
                "'flox search' requires at least one search term, run 'flox search -h' for more information."
            )
        }

        let search_params = construct_search_params(&self.search_term, &flox)?;
        let search_params_json = serde_json::to_string(&search_params)?;

        let output = Command::new(PKGDB_BIN.as_str())
            .arg("search")
            .arg("--quiet")
            .arg(search_params_json)
            .stderr(std::process::Stdio::inherit())
            .output()?;

        if output.status.success() {
            // FIXME: We may have warnings on `stderr` even with a successful call to `pkgdb`.
            //        We aren't checking that at all at the moment because better overall error handling
            //        is coming in a later PR.
            let search_results = SearchResults::try_from(output.stdout.as_slice())?;
            render_search_results(search_results, self.json)?;
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stdout);
            bail!(
                "pkgdb exited with status code {}: {}",
                output.status.code().unwrap_or(-1),
                err_msg
            );
        }
    }
}

fn construct_search_params(search_term: &Option<String>, flox: &Flox) -> Result<SearchParams> {
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
    let registry = Registry {
        inputs,
        priority,
        defaults: RegistryDefaults::default(),
    };

    // We've already checked that the search term is Some(_)
    let query = Query::from_str(search_term.as_ref().unwrap())?;

    Ok(SearchParams {
        registry,
        query,
        ..SearchParams::default()
    })
}

// This is likely to change significantly after the output format of search results is specced out
fn render_search_results(search_results: SearchResults, as_json: bool) -> Result<()> {
    if as_json {
        let json = serde_json::to_string(&search_results.results)?;
        println!("{}", json);
        return Ok(());
    }
    let summarized_results = search_results
        .results
        .iter()
        .map(|r| {
            let path_components = r.attr_path.clone();
            let flake_attr = if !HIDDEN_CHANNELS.contains_key(r.input.as_str()) {
                vec![r.input.clone(), path_components.join_with(".").to_string()]
                    .join_with("#")
                    .to_string()
            } else {
                path_components.join_with(".").to_string()
            };
            let d = if let Some(ref d) = r.description {
                // Some package descriptions contain newline characters
                // which breaks formatting
                d.replace('\n', " ")
            } else {
                "<no description provided>".into()
            };
            (flake_attr, d)
        })
        .collect::<Vec<_>>();
    let attr_col_width = summarized_results
        .iter()
        .fold(0, |cw, (attr, _)| usize::max(cw, attr.len()));
    // Depending on the search query there could be a ton of results, better to
    // do buffered writes than to lock `stdout` on every write.
    let mut writer = BufWriter::new(std::io::stdout());
    for (attr, desc) in summarized_results.iter() {
        writer.write_fmt(format_args!(
            "{:<width$}  {}\n",
            attr,
            desc,
            width = attr_col_width
        ))?;
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
