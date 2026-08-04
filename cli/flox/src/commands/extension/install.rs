use std::path::PathBuf;

use anyhow::{Result, bail};
use beta::extensions;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use crate::commands::SHELL_COMPLETION_DIR;
use crate::subcommand_metric;
use crate::utils::message;

#[derive(Debug, Bpaf, Clone)]
pub struct Install {
    /// Install from an explicit local path (alternative to '.')
    #[bpaf(long, argument("PATH"), complete_shell(SHELL_COMPLETION_DIR))]
    from_path: Option<PathBuf>,

    /// Pin to a specific git ref (tag like 'v1.2.3' or commit SHA prefix)
    #[bpaf(long, argument("REF"))]
    pin: Option<String>,

    /// Overwrite an existing install (install) or override pin and re-fetch (upgrade)
    #[bpaf(long, switch)]
    force: bool,

    /// Spec to install — '.' for the current directory, or 'owner/repo'
    /// for a GitHub source. Use --from-path PATH for an explicit local path.
    #[bpaf(positional("SPEC"), fallback(String::new()))]
    spec: String,
}

impl Install {
    #[instrument(name = "extensions::install", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("extensions::install");

        match (self.spec.as_str(), self.from_path.as_ref()) {
            (".", None) => {
                if self.pin.is_some() {
                    bail!("--pin only applies to GitHub sources, not '.'");
                }
                let cwd = std::env::current_dir()?;
                let ext = extensions::install_local(&flox, &cwd, self.force)?;
                message::updated(format!(
                    "Installed flox-{} (local) -> {}",
                    ext.name,
                    ext.install_dir.display()
                ));
            },
            ("", Some(p)) => {
                if self.pin.is_some() {
                    bail!("--pin only applies to GitHub sources, not --from-path");
                }
                let ext = extensions::install_local(&flox, p, self.force)?;
                message::updated(format!(
                    "Installed flox-{} (local) -> {}",
                    ext.name,
                    ext.install_dir.display()
                ));
            },
            (".", Some(_)) => bail!("specify either '.' or --from-path, not both"),
            ("", None) => {
                bail!("usage: flox extension install . | --from-path PATH | OWNER/REPO")
            },
            (spec, Some(_)) => bail!("--from-path is mutually exclusive with SPEC '{spec}'"),
            (spec, None) if looks_like_github_spec(spec) => {
                let ext = extensions::install_github(&flox, spec, self.pin.as_deref(), self.force)
                    .await?;
                let suffix = match (ext.state.tag.as_str(), ext.state.commit.get(..8)) {
                    (tag, _) if !tag.is_empty() => format!("@{tag}"),
                    (_, Some(sha)) => format!("@{sha}"),
                    _ => String::new(),
                };
                message::updated(format!(
                    "Installed flox-{} ({}{})",
                    ext.name, ext.state.kind, suffix
                ));
            },
            (spec, None) => {
                bail!("unsupported spec: '{spec}' — use '.', --from-path PATH, or 'owner/repo'")
            },
        }
        Ok(())
    }
}

fn looks_like_github_spec(s: &str) -> bool {
    s.contains('/') && !s.starts_with('.') && !s.starts_with('/')
}
