//! Modiule for all defined environment variables to
//! reduce the number of magic strings

use anyhow::Result;
use std::collections::HashMap;
pub static NIX_BIN: &str = env!("NIX_BIN");
pub static FLOX_SH: &str = env!("FLOX_SH");

/// Environment variable key for the GitHub Api Key
pub static GITHUB_TOKEN: &str = "GITHUB_TOKEN";

pub fn build_flox_env() -> Result<HashMap<String, String>> {
    let home = env!("HOME");

    let mut env_map: HashMap<String, String> = HashMap::new();

    env_map.insert("NIX_REMOTE".to_string(), "daemon".to_string());
    // figure out how to get this from the flox environment
    env_map.insert(
        "NIX_SSL_CERT_FILE".to_string(),
        format!(
            "{}/etc/ssl/certs/ca-bundle.crt",
            "/nix/store/3rj7pc0phyva0g2nry0an4sjpjmmfxds-nss-cacert-3.80"
        ),
    );
    env_map.insert(
        "NIX_USER_CONF_FILES".to_string(),
        format!("{}/.config/flox/nix.conf", home),
    );
    env_map.insert(
        "GIT_CONFIG_SYSTEM".to_string(),
        format!("{}/.config/flox/gitconfig", home),
    );

    Ok(env_map)
}
