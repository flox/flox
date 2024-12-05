use std::convert::Infallible;
use std::path::PathBuf;

use flox_rust_sdk::providers::container_builder::{ContainerBuilder, ContainerSource};

#[allow(unused)]
struct ContainerizeProxy {
    environment_path: PathBuf,
}

impl ContainerBuilder for ContainerizeProxy {
    type Error = Infallible;

    fn create_container_source(
        &self,
        _name: impl AsRef<str>,
        _tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error> {
        todo!("🚧 MacOS container builder in construction 🚧")
    }
}
