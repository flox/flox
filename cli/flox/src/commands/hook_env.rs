use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;

#[derive(Debug, Clone, Bpaf)]
pub struct HookEnv {
    /// Shell to emit hook-env code for (bash, zsh, fish, tcsh)
    #[bpaf(long("shell"), argument("SHELL"))]
    shell: String,
}

impl HookEnv {
    pub fn handle(self, flox: Flox) -> Result<()> {
        let _shell = self.shell; // used by future implementation
        if !flox.features.auto_activate {
            bail!(
                "'hook-env' requires the auto_activate feature flag. Set FLOX_FEATURES_AUTO_ACTIVATE=true."
            );
        }
        // No-op: future tickets implement directory scanning,
        // trust/preference filtering, and environment activation.
        Ok(())
    }
}
