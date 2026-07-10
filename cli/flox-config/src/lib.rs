//! Layered configuration shared by flox CLIs.
//!
//! Resolution order (lowest to highest precedence):
//! defaults, system config (`/etc/flox/flox.toml`), user config
//! (`$XDG_CONFIG_HOME/flox/flox.toml`), `FLOX_*` environment variables.
