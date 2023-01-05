use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;

use crate::config::Feature;
use crate::{flox_forward, should_flox_forward};

#[derive(Bpaf, Clone)]
pub struct ChannelArgs {}

impl ChannelCommands {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match self {
            _ if should_flox_forward(Feature::Env)? => flox_forward(&flox).await?,

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

    /// unsubscribe from channel
    #[bpaf(command)]
    Unsubscribe {
        #[bpaf(positional("channel"))]
        channel: ChannelRef,
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
    Channels {},
}

pub type ChannelRef = String;
pub type Url = String;
