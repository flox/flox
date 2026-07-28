use std::path::PathBuf;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use crate::beta::extensions;
use crate::commands::SHELL_COMPLETION_DIR;
use crate::subcommand_metric;
use crate::utils::message;

#[derive(Debug, Bpaf, Clone)]
pub struct Install {
    /// Install from an explicit local path (alternative to '.')
    #[bpaf(long, argument("PATH"), complete_shell(SHELL_COMPLETION_DIR))]
    from_path: Option<PathBuf>,

    /// Overwrite an existing install
    #[bpaf(long, switch)]
    force: bool,

    /// Source to install — '.' for the current directory. Use
    /// --from-path PATH for an explicit local path.
    #[bpaf(positional("SOURCE"), fallback(String::new()))]
    source: String,
}

impl Install {
    #[instrument(name = "extensions::install", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("extensions::install");

        match (self.source.as_str(), self.from_path.as_ref()) {
            (".", None) => {
                let cwd = std::env::current_dir()?;
                let ext = extensions::install_local(&flox, &cwd, self.force)?;
                message::updated(format!(
                    "Installed flox-{} -> {}",
                    ext.name,
                    ext.install_dir.display()
                ));
            },
            ("", Some(p)) => {
                let ext = extensions::install_local(&flox, p, self.force)?;
                message::updated(format!(
                    "Installed flox-{} -> {}",
                    ext.name,
                    ext.install_dir.display()
                ));
            },
            (".", Some(_)) => bail!("specify either '.' or --from-path, not both"),
            ("", None) => {
                bail!("usage: flox extension install . | --from-path PATH")
            },
            (source, Some(_)) => bail!("--from-path is mutually exclusive with SOURCE '{source}'"),
            (source, None) => {
                bail!("unsupported source: '{source}' — use '.' or --from-path PATH")
            },
        }
        Ok(())
    }
}
