use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;

use crate::config::features::Feature;
use crate::flox_forward;

#[derive(Bpaf, Clone)]
pub struct ChannelArgs {}

impl ChannelCommands {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match self {
            _ if Feature::Env.is_forwarded()? => flox_forward(&flox).await?,

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
        /// If ommited, flow will prompt for the name interactively
        #[bpaf(positional("channel"), optional)]
        channel: Option<ChannelRef>,
    },

    /// search packages in subscribed channels
    #[bpaf(command)]
    Search {
        #[bpaf(short, long, argument("channel"))]
        channel: Option<ChannelRef>,

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
