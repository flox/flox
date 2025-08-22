use std::path::PathBuf;

use anyhow::Context;
use bpaf::Bpaf;

#[derive(Debug, Clone, Bpaf)]
pub struct PathHash {
    /// The path to compute the hash of. If not specified, we fall back
    /// to the hash of `$FLOX_ENV`.
    #[bpaf(positional("path"))]
    pub path: Option<PathBuf>,
}

impl PathHash {
    pub fn handle(&self) -> Result<(), anyhow::Error> {
        let path = if let Some(path) = self.path.as_ref() {
            path.clone()
        } else {
            let flox_env = std::env::var("FLOX_ENV").context("FLOX_ENV not set")?;
            PathBuf::from(flox_env)
        };
        let hash = flox_core::path_hash(&path);
        println!("{hash}");
        Ok(())
    }
}
