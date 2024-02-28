//! Modiule for all defined environment variables to
//! reduce the number of magic strings

use std::collections::HashMap;
use std::env;
pub static NIX_BIN: &str = env!("NIX_BIN");

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
///
/// * `set_all` - Set all environment variables irrespective of
///               their presence in the current environment.
pub fn default_nix_subprocess_env(set_all: bool) -> HashMap<&'static str, String> {
    let mut env_map: HashMap<&str, String> = HashMap::new();

    // respect SSL_CERT_FILE, but if it isn't set, use buildtime NIXPKGS_CACERT_BUNDLE_CRT
    let ssl_cert_file = match env::var("SSL_CERT_FILE") {
        Ok(v) => v,
        Err(_) => {
            let nixpkgs_cacert_bundle_crt = env!("NIXPKGS_CACERT_BUNDLE_CRT");
            env_map.insert(
                "SSL_CERT_FILE",
                nixpkgs_cacert_bundle_crt.to_string(),
            );
            nixpkgs_cacert_bundle_crt.to_string()
        },
    };

    if set_all || env::var("NIX_SSL_CERT_FILE").is_err() {
        env_map.insert("NIX_SSL_CERT_FILE", ssl_cert_file);
    }

    #[cfg(target_os = "macos")]
    {
        if set_all || env::var("NIX_COREFOUNDATION_RPATH").is_err() {
            env_map.insert(
                "NIX_COREFOUNDATION_RPATH",
                env!("NIX_COREFOUNDATION_RPATH").to_string(),
            );
        }
        if set_all || env::var("PATH_LOCALE").is_err() {
            env_map.insert("PATH_LOCALE", env!("PATH_LOCALE").to_string());
        }
    }

    #[cfg(target_os = "linux")]
    {
        if set_all || env::var("LOCALE_ARCHIVE").is_err() {
            env_map.insert(
                "LOCALE_ARCHIVE",
                env!("LOCALE_ARCHIVE").to_string(),
            );
        }
    }

    env_map.insert(
        "FLOX_VERSION",
        crate::flox::FLOX_VERSION.to_string(),
    );

    // For now these variables are managed in bash
    // let home = env!("HOME");
    // env_map.insert(
    //     "NIX_USER_CONF_FILES".to_string(),
    //     format!("{}/.config/flox/nix.conf", home),
    // );

    env_map
}
