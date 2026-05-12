use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use shell_gen::Shell;

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
        // Temporary: set _FLOX_HOOK_FIRED so we can verify the hook fires.
        // This will be replaced by real environment activation logic.
        let cwd = std::env::current_dir()?.to_string_lossy().to_string();
        match self.shell {
            Shell::Bash | Shell::Zsh => println!(r#"export _FLOX_HOOK_FIRED="{cwd}";"#),
            Shell::Fish => println!(r#"set -gx _FLOX_HOOK_FIRED "{cwd}";"#),
            Shell::Tcsh => println!("setenv _FLOX_HOOK_FIRED {cwd};"),
        }
        Ok(())
    }
}
