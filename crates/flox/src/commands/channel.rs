use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::{flox::Flox, nix::command_line::NixCommandLine, prelude::Stability};

use crate::{config::Config, flox_forward};

#[derive(Bpaf)]
pub struct ChannelArgs {
    #[bpaf(external(channel_commands))]
    command: ChannelCommands,
}

impl ChannelArgs {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match &self.command {
            _ if !Config::preview_enabled()? => flox_forward().await?,
            _ => todo!(),
        }

        Ok(())
    }
}

#[derive(Bpaf, Clone)]
#[bpaf(adjacent)]
pub enum ChannelCommands {
    /// subscribe to channel URL
    #[bpaf(command)]
    Subscribe {
        #[bpaf(positional)]
        name: Option<ChannelRef>,
        #[bpaf(positional)]
        url: Option<Url>,
    },

    /// unsubscribe from channel
    #[bpaf(command)]
    Unsubscribe {
        #[bpaf(positional)]
        channel: ChannelRef,
    },

    /// search packages in subscribed channels
    #[bpaf(command)]
    Search {
        #[bpaf(short, long)]
        channel: ChannelRef,
        #[bpaf(positional)]
        search_term: Option<String>,
    },

    /// list all subscribed channels
    #[bpaf(command)]
    Channels {},
}

pub type ChannelRef = String;
pub type Url = String;
