use anyhow::Result;
use clap::Parser;
use flox_rust_sdk::providers::initializers;

mod build;
mod config;
mod utils;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub(crate) struct FloxArgs {
    #[clap(subcommand, help = "Initialize a flox project")]
    init: InitializeAction,
}

#[derive(clap::Subcommand, Debug)]
pub(crate) enum InitializeAction {
    Init {
        #[clap(value_parser, help = "The package name you are trying to initialize")]
        package_name: String,
        #[clap(value_parser, help = "The builder you would like to use.")]
        builder: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = FloxArgs::parse();
    println!("{:?}", args);

    match args.init {
        InitializeAction::Init {
            package_name,
            builder,
        } => {
            initializers::get_provider()
                .await?
                .init(&package_name, &builder.into())
                .await?;
        }
    }

    Ok(())
}

// #[cfg(test)]
// mod tests {
//     use clap::Parser;
//     use crate::FloxArgs;

//     use super::*;

//     #[tokio::test]
//     async fn test_create() {
//         let args = FloxArgs::parse();
//         println!("{:?}",args);
//     }
// }
