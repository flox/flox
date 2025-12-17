use std::collections::HashMap;
use std::env;
use std::io::Stderr;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};

pub mod active_environments;
pub mod colors;
pub mod dialog;
pub mod didyoumean;
pub mod errors;
pub mod init;
pub mod message;
pub mod metrics;
pub mod openers;
pub mod search;
pub mod tracing;
pub mod update_notifications;

pub static TERMINAL_STDERR: LazyLock<Mutex<Stderr>> =
    LazyLock::new(|| Mutex::new(std::io::stderr()));
/// Timeout used for network operations that run after the main flox command has
/// completed.
///
/// This is used for metrics submission and checking for updates.
pub const TRAILING_NETWORK_CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// Explicitly set environment for nix calls
///
/// Nixpkgs itself is broken in that the packages it creates depends
/// upon a variety of environment variables at runtime.  On NixOS
/// these are convenient to set on a system-wide basis but that
/// essentially masks the problem, and it's not uncommon to see Nix
/// packages trip over the absence of environment variables when
/// invoked on other Linux distributions.
///
/// For flox specifically, set Nix-provided defaults for certain
/// environment variables that we know to be required on the various
/// operating systems.
/// Setting buildtime variants of these environment variables
/// will bundle them in flox' package closure
/// and ensure that subprocesses are run with valid known values.
pub fn default_nix_env_vars() -> std::collections::HashMap<&'static str, String> {
    let mut env_map: HashMap<&str, String> = HashMap::new();

    // use buildtime NIXPKGS_CACERT_BUNDLE_CRT
    let ssl_cert_file = match env::var("SSL_CERT_FILE") {
        Ok(v) => v,
        Err(_) => {
            let nixpkgs_cacert_bundle_crt = env!("NIXPKGS_CACERT_BUNDLE_CRT");
            env_map.insert("SSL_CERT_FILE", nixpkgs_cacert_bundle_crt.to_string());
            nixpkgs_cacert_bundle_crt.to_string()
        },
    };

    env_map.insert("NIX_SSL_CERT_FILE", ssl_cert_file);

    // on macos use buildtime PATH_LOCALE
    #[cfg(target_os = "macos")]
    {
        env_map.insert("PATH_LOCALE", env!("PATH_LOCALE").to_string());
    }

    // on linux use buildtime LOCALE_ARCHIVE
    #[cfg(target_os = "linux")]
    {
        env_map.insert("LOCALE_ARCHIVE", env!("LOCALE_ARCHIVE").to_string());
    }

    env_map
}

/// Set the default nix environment variables for the current process
///
/// SAFETY: called once, prior to possible concurrent access to env
pub fn populate_default_nix_env_vars() {
    let env_map = default_nix_env_vars();
    for (key, value) in env_map {
        unsafe { env::set_var(key, value) }
    }
}

pub fn bail_on_v2_manifest_without_feature_flag(
    flox: &Flox,
    concrete_env: &ConcreteEnvironment,
) -> Result<(), anyhow::Error> {
    if flox.features.outputs {
        return Ok(());
    }
    let is_v1 = match concrete_env {
        ConcreteEnvironment::Path(env) => env.manifest(flox)?.version() == 1,
        ConcreteEnvironment::Managed(env) => env.manifest(flox)?.version() == 1,
        ConcreteEnvironment::Remote(env) => env.manifest(flox)?.version() == 1,
    };
    if is_v1 {
        return Ok(());
    }
    anyhow::bail!("manifest schema version 2 is only allowed with FLOX_FEATURES_OUTPUTS=1");
}
