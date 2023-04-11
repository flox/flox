use anyhow::Result;
use bpaf::Bpaf;
use derive_more::Display;
use flox_rust_sdk::flox::Flox;
use itertools::Itertools;
use serde_json::json;

use crate::config::features::Feature;
use crate::flox_forward;
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

impl ChannelCommands {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match self {
            _ if Feature::Channels.is_forwarded()? => flox_forward(&flox).await?,
            ChannelCommands::Channels { json } => {
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

                if *json {
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
            },
            _ => todo!(),
        }

        Ok(())
    }
}

#[derive(Bpaf, Clone)]
pub enum ChannelCommands {
    /// subscribe to channel URL
    #[bpaf(command)]
    Subscribe {
        #[bpaf(positional("name"))]
        name: Option<ChannelRef>,
        #[bpaf(positional("url"))]
        url: Option<Url>,
    },

    /// unsubscribe from a channel
    #[bpaf(command)]
    Unsubscribe {
        /// channel name to unsubscribe.
        /// If omitted, flow will prompt for the name interactively
        #[bpaf(positional("channel"), optional)]
        channel: Option<ChannelRef>,
    },

    /// search packages in subscribed channels
    #[bpaf(command)]
    Search {
        #[bpaf(short, long, argument("channel"))]
        channel: Vec<ChannelRef>,

        /// print search as JSON
        #[bpaf(long)]
        json: bool,

        #[bpaf(positional("search term"))]
        search_term: Option<String>,
    },

    /// list all subscribed channels
    #[bpaf(command)]
    Channels {
        /// print channels as JSON
        #[bpaf(long)]
        json: bool,
    },
}

pub type ChannelRef = String;
pub type Url = String;
