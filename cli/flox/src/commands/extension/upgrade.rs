use anyhow::{Result, bail};
use beta::extensions::{self, DryRunStatus, UpgradeStatus};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use crate::subcommand_metric;
use crate::utils::message;

#[derive(Debug, Bpaf, Clone)]
pub struct Upgrade {
    /// Upgrade every installed extension instead of one
    #[bpaf(long, switch)]
    all: bool,

    /// Resolve but don't mutate state.toml or the install dir
    #[bpaf(long, switch)]
    dry_run: bool,

    /// Override pin and re-fetch (single-item or --all)
    #[bpaf(long, switch)]
    force: bool,

    /// Name of the extension to upgrade (omit when --all is given)
    #[bpaf(positional("NAME"), fallback(String::new()))]
    name: String,
}

impl Upgrade {
    #[instrument(name = "extensions::upgrade", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("extensions::upgrade");

        match (self.all, self.name.is_empty()) {
            // --all with NAME — ambiguous, reject.
            (true, false) => bail!("--all and NAME are mutually exclusive"),
            // Neither flag nor NAME — nothing to do.
            (false, true) => {
                bail!("usage: flox extension upgrade [--all] [--dry-run] [--force] [NAME]")
            },
            // --all without NAME — iterate every installed extension.
            (true, true) => self.run_all(flox).await,
            // NAME without --all — single-item path.
            (false, false) => self.run_one(flox).await,
        }
    }

    async fn run_one(&self, flox: Flox) -> Result<()> {
        if self.dry_run {
            match extensions::upgrade_dry_run(&flox, &self.name, self.force).await? {
                DryRunStatus::WouldUpgrade { from, to } => {
                    message::info(format!(
                        "flox-{}: would upgrade {} -> {}",
                        self.name,
                        super::format::short_sha(&from),
                        super::format::short_sha(&to)
                    ));
                },
                DryRunStatus::AlreadyCurrent => {
                    message::info(format!("flox-{} is already current", self.name));
                },
                DryRunStatus::Pinned => {
                    message::info(format!(
                        "flox-{} is pinned; pass --force to override",
                        self.name
                    ));
                },
            }
            return Ok(());
        }

        match extensions::upgrade(&flox, &self.name, self.force).await? {
            UpgradeStatus::Upgraded { from, to } => {
                message::updated(format!(
                    "Upgraded flox-{}: {} -> {}",
                    self.name,
                    super::format::short_sha(&from),
                    super::format::short_sha(&to)
                ));
            },
            UpgradeStatus::AlreadyCurrent => {
                message::info(format!(
                    "flox-{} is already at the latest commit",
                    self.name
                ));
            },
            UpgradeStatus::Pinned => {
                bail!(
                    "flox-{} is pinned; pass --force to override and re-fetch",
                    self.name
                );
            },
        }
        Ok(())
    }

    async fn run_all(&self, flox: Flox) -> Result<()> {
        let extensions_listed = extensions::list(&flox)?;
        if extensions_listed.is_empty() {
            message::plain("No extensions installed.");
            return Ok(());
        }
        println!("{}", super::format::render_header());

        if self.dry_run {
            for result in extensions::upgrade_all_dry_run(&flox, self.force).await? {
                let ext = extensions_listed.iter().find(|e| e.name == result.name);
                let mut row = match ext {
                    Some(e) => super::format::row_from_extension(e),
                    None => super::format::row_for_unknown(&result.name),
                };
                row.status = Some(match &result.outcome {
                    Ok(DryRunStatus::WouldUpgrade { from, to }) => {
                        format!(
                            "would upgrade {} -> {}",
                            super::format::short_sha(from.as_str()),
                            super::format::short_sha(to.as_str())
                        )
                    },
                    Ok(DryRunStatus::AlreadyCurrent) => "up-to-date".to_string(),
                    Ok(DryRunStatus::Pinned) => "pinned (skip)".to_string(),
                    Err(e) => format!("error: {e}"),
                });
                println!("{}", super::format::render_row(&row));
            }
            return Ok(());
        }

        for result in extensions::upgrade_all(&flox, self.force).await? {
            let ext = extensions_listed.iter().find(|e| e.name == result.name);
            let mut row = match ext {
                Some(e) => super::format::row_from_extension(e),
                None => super::format::row_for_unknown(&result.name),
            };
            row.status = Some(match &result.outcome {
                Ok(UpgradeStatus::Upgraded { from, to }) => {
                    format!(
                        "upgraded {} -> {}",
                        super::format::short_sha(from.as_str()),
                        super::format::short_sha(to.as_str())
                    )
                },
                Ok(UpgradeStatus::AlreadyCurrent) => "up-to-date".to_string(),
                Ok(UpgradeStatus::Pinned) => "pinned (skip)".to_string(),
                Err(e) => format!("error: {e}"),
            });
            println!("{}", super::format::render_row(&row));

            // Pinned-skip log line per research doc §2.9.
            if let Ok(UpgradeStatus::Pinned) = &result.outcome {
                let tag = ext
                    .and_then(|e| (!e.state.tag.is_empty()).then(|| e.state.tag.clone()))
                    .unwrap_or_else(|| "<unknown>".to_string());
                message::info(format!(
                    "skipping '{}' (pinned to {}); pass --force to override",
                    result.name, tag
                ));
            }
        }
        Ok(())
    }
}
