use clap::Parser;

use utils::*;

mod config;
mod build;
mod utils;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub(crate) struct FloxArgs {
    #[clap(subcommand)]
    create: CreateAction
}

#[derive(clap::Subcommand, Debug)]
pub (crate) enum BuildAction {
    PathSelector {
        #[clap(value_parser)]
        path: String
    }
}


#[tokio::main]
async fn main() {
    let args = FloxArgs::parse();
    println!("{:?}",args);
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use super::app::*;

    #[tokio::test]
    async fn test_create() {
        let args = FloxArgs::parse();
        println!("{:?}",args);
    }
}
