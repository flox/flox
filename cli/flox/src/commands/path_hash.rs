use std::path::PathBuf;

use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;

use super::{EnvironmentSelect, environment_select};
use crate::utils::message;

#[derive(Debug, Clone, Bpaf)]
pub struct PathHash {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    pub environment: EnvironmentSelect,

    /// Explicit path to compute the hash of (overrides environment selection)
    #[bpaf(positional("PATH"), optional)]
    pub path: Option<PathBuf>,
}

impl PathHash {
    pub fn handle(&self, flox: Flox) -> Result<(), anyhow::Error> {
        let path_to_hash = if let Some(path) = &self.path {
            match std::fs::canonicalize(path) {
                Ok(canonical) => canonical,
                Err(err) => {
                    message::warning(format!(
                        "couldn't canonicalize path {}: {}",
                        path.display(),
                        err
                    ));
                    path.clone()
                },
            }
        } else {
            let concrete_env = self
                .environment
                .detect_concrete_environment(&flox, "Environment path to hash")?;
            concrete_env.dot_flox_path().to_path_buf()
        };

        let hash = flox_core::path_hash(&path_to_hash);
        println!("{hash}");
        Ok(())
    }
}
