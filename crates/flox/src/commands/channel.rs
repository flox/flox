use anyhow::{bail, Context, Result};
use bpaf::Bpaf;
use derive_more::Display;
use flox_rust_sdk::flox::{Flox, DEFAULT_OWNER};
use flox_rust_sdk::nix::command::FlakeMetadata;
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::nix::flake_ref::git_service::{GitServiceAttributes, GitServiceRef};
use flox_rust_sdk::nix::flake_ref::FlakeRef;
use flox_rust_sdk::nix::RunJson;
use flox_rust_sdk::providers::git::GitCommandProvider;
use itertools::Itertools;
use regex::Regex;
use serde_json::json;

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

impl Search {
    pub async fn handle(self, flox: Flox) -> Result<()> {
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
            .floxmeta::<GitCommandProvider>(DEFAULT_OWNER)
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
        let floxmeta = flox
            .floxmeta::<GitCommandProvider>(DEFAULT_OWNER)
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
