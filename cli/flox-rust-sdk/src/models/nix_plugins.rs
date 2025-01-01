use std::env;
use std::sync::LazyLock;

/// Directory containeing Flox' nix plugins
pub static NIX_PLUGINS: LazyLock<String> =
    LazyLock::new(|| env::var("NIX_PLUGINS").unwrap_or(env!("NIX_PLUGINS").to_string()));
