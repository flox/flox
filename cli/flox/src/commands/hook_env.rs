use std::borrow::Cow;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use shell_gen::Shell;

use crate::subcommand_metric;

#[derive(Debug, Clone, Bpaf)]
pub struct HookEnv {
    /// Shell to emit hook-env code for (bash, zsh, fish, tcsh)
    #[bpaf(long("shell"), argument("SHELL"))]
    shell: Shell,
}

impl HookEnv {
    pub fn handle(self, flox: Flox) -> Result<()> {
        if !flox.features.auto_activate {
            bail!(
                "'hook-env' requires the auto_activate feature flag. Set FLOX_FEATURES_AUTO_ACTIVATE=true."
            );
        }

        // TODO: when we add auto-activation logic, we should probably skip this
        // on the fast path and only add it when we're making a meaningful change.
        // We could also consider counting unique environments or something
        // instead of recording every single run of this command.
        subcommand_metric!("hook-env");

        // Temporary: set _FLOX_HOOK_FIRED so we can verify the hook fires.
        // This will be replaced by real environment activation logic.
        let cwd = std::env::current_dir()?.to_string_lossy().to_string();
        let escaped_cwd = shell_escape::escape(Cow::Borrowed(&cwd));
        match self.shell {
            Shell::Bash | Shell::Zsh => println!("export _FLOX_HOOK_FIRED={escaped_cwd};"),
            Shell::Fish => println!("set -gx _FLOX_HOOK_FIRED {escaped_cwd};"),
            Shell::Tcsh => println!("setenv _FLOX_HOOK_FIRED {escaped_cwd};"),
        }
        Ok(())
    }
}
