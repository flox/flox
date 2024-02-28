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
/// Setting buildtime variants of these environment variables
/// will bundle include them in flox' pacakge closure
/// and ennsure that subprocesses are run with valid konwn values.
pub fn default_nix_subprocess_env() -> HashMap<&'static str, String> {
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

    // on macos use buildtime NIX_COREFOUNDATION_RPATH and PATH_LOCALE
    #[cfg(target_os = "macos")]
    {
        env_map.insert(
            "NIX_COREFOUNDATION_RPATH",
            env!("NIX_COREFOUNDATION_RPATH").to_string(),
        );
        env_map.insert("PATH_LOCALE", env!("PATH_LOCALE").to_string());
    }

    // on linux use buildtime LOCALE_ARCHIVE
    #[cfg(target_os = "linux")]
    {
        env_map.insert("LOCALE_ARCHIVE", env!("LOCALE_ARCHIVE").to_string());
    }

    // TODO: remove `FLOX_VERSION`.
    // Not removing just yet, as I'm not sure why it's here.
    env_map.insert("FLOX_VERSION", crate::flox::FLOX_VERSION.to_string());

    env_map
}
