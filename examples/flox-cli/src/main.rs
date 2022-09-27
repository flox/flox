use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub(crate) struct FloxArgs {
    #[clap(subcommand)]
    create: CreateAction
}

#[derive(clap::Subcommand, Debug)]
pub (crate) enum CreateAction {
    Install {
        #[clap(value_parser)]
        package_name: String
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
