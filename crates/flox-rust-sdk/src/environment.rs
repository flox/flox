//! Modiule for all defined environment variables to
//! reduce the number of magic strings

use std::collections::HashMap;
use std::env;
pub static NIX_BIN: &str = env!("NIX_BIN");

/// Environment variable key for the GitHub Api Key
pub static GITHUB_TOKEN: &str = "GITHUB_TOKEN";

pub fn build_flox_env() -> HashMap<String, String> {
    let mut env_map: HashMap<String, String> = HashMap::new();

    /*
     * Nixpkgs itself is broken in that the packages it creates depends
     * upon a variety of environment variables at runtime.  On NixOS
     * these are convenient to set on a system-wide basis but that
     * essentially masks the problem, and it's not uncommon to see Nix
     * packages trip over the absence of environment variables when
     * invoked on other Linux distributions.
     *
     * For flox specifically, set Nix-provided defaults for certain
     * environment variables that we know to be required on the various
     * operating systems.
     */

    // respect SSL_CERT_FILE, but if it isn't set, use buildtime NIXPKGS_CACERT_BUNDLE_CRT
    let ssl_cert_file = match env::var("SSL_CERT_FILE") {
        Ok(v) => v,
        Err(_) => {
            let nixpkgs_cacert_bundle_crt = env!("NIXPKGS_CACERT_BUNDLE_CRT");
            env_map.insert(
                "SSL_CERT_FILE".to_string(),
                nixpkgs_cacert_bundle_crt.to_string(),
            );
            nixpkgs_cacert_bundle_crt.to_string()
        }
    };

    if env::var("NIX_SSL_CERT_FILE").is_err() {
        env_map.insert("NIX_SSL_CERT_FILE".to_string(), ssl_cert_file);
    }

    #[cfg(target_os = "macos")]
    {
        if env::var("NIX_COREFOUNDATION_RPATH").is_err() {
            env_map.insert(
                "NIX_COREFOUNDATION_RPATH".to_string(),
                env!("NIX_COREFOUNDATION_RPATH").to_string(),
            );
        }
        if env::var("PATH_LOCALE").is_err() {
            env_map.insert("PATH_LOCALE".to_string(), env!("PATH_LOCALE").to_string());
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Err(_) = env::var("LOCALE_ARCHIVE") {
            env_map.insert(
                "LOCALE_ARCHIVE".to_string(),
                env!("LOCALE_ARCHIVE").to_string(),
            );
        }
    }

    env_map.insert("NIX_REMOTE".to_string(), "daemon".to_string());

    // For now these variables are managed in bash
    // let home = env!("HOME");
    // env_map.insert(
    //     "NIX_USER_CONF_FILES".to_string(),
    //     format!("{}/.config/flox/nix.conf", home),
    // );
    // env_map.insert(
    //     "GIT_CONFIG_SYSTEM".to_string(),
    //     format!("{}/.config/flox/gitconfig", home),
    // );

    env_map
}
