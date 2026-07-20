//! Layered configuration shared by flox CLIs.
//!
//! Resolution order (lowest to highest precedence):
//! defaults, system config (`/etc/flox/flox.toml`), user config
//! (`$XDG_CONFIG_HOME/flox/flox.toml`), `FLOX_*` environment variables.

mod config;
mod load;
mod write;

pub use config::{
    AuthnMode,
    AutoActivate,
    AutoActivationPreference,
    Config,
    EnvironmentPromptConfig,
    EnvironmentTrust,
    FLOX_CONFIG_FILE,
    FLOX_DIR_NAME,
    FloxConfig,
    InstallerChannel,
    PublishConfig,
    SearchLimit,
    TokenStorageMode,
};
pub use write::ReadWriteError;
