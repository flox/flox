use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, Result};
use config::{Config as HierarchicalConfig, Environment};
use itertools::{Either, Itertools};
use tracing::debug;
use xdg::BaseDirectories;

use crate::config::{Config, FLOX_CONFIG_FILE, FLOX_DIR_NAME};

const FLOX_CONFIG_DIR_VAR: &str = "FLOX_CONFIG_DIR";

/// Creates a [Config] from the environment and config files
pub(crate) fn parse() -> Result<Config> {
    let base_directories = BaseDirectories::with_prefix(FLOX_DIR_NAME);
    parse_with(
        &base_directories,
        &locate_user_config_dir(&base_directories)?,
        locate_system_config_dir()?.as_deref(),
        env::vars(),
    )
}

pub(crate) fn parse_with(
    flox_dirs: &BaseDirectories,
    user_config_dir: &Path,
    system_config_dir: Option<&Path>,
    env: impl IntoIterator<Item = (String, String)>,
) -> Result<Config> {
    let final_config = raw_config_from_parts(flox_dirs, user_config_dir, system_config_dir, env)?;

    let cli_config: Config = final_config
        .to_owned()
        .try_deserialize()
        .context("Could not parse config")?;
    Ok(cli_config)
}

/// Locates the system wide flox config dir.
///
/// By default that is `/etc`.
/// The directory can be overridden by the user via the environment variable
/// `$FLOX_SYSTEM_CONFIG_DIR`.
///
/// IFF `$FLOX_SYSTEM_CONFIG_DIR` is set to an empty string, system config will be ignored.
fn locate_system_config_dir() -> Result<Option<PathBuf>> {
    // TODO: make constant
    match env::var("FLOX_SYSTEM_CONFIG_DIR").ok() {
        Some(path) if path.is_empty() => Ok(None),
        Some(path) => Ok(Some(path.into())),
        None => Ok(Some(PathBuf::from("/etc"))),
    }
}

/// Locates the user specific config dir.
///
/// The implementation probes for config files in relevant xdg dirs in order:
/// $XDG_CONFIG_HOME/flox/flox.toml
/// for dir in $XDG_CONFIG_DIRS: $dir/flox/flox.toml
///
/// The first occurrence determines the config_dir.
/// If no config file was found, this function defaults to $XDG_CONFIG_HOME/flox.
///
/// IFF $FLOX_CONFIG_DIR is set to a non-empty string,
/// its value will be used as the user config dir instead.
fn locate_user_config_dir(flox_dirs: &BaseDirectories) -> Result<PathBuf> {
    let user_config_dir = env::var(FLOX_CONFIG_DIR_VAR).ok();

    let config_dir = match user_config_dir {
        Some(path) if !path.is_empty() => {
            debug!(?path, "Global config directory overridden");
            fs::create_dir_all(&path)
                .context(format!("Could not create config directory: {path:?}"))?;
            path.into()
        },
        None | Some(_ /* empty */) => {
            let config_dir = if let Some(existing_xdg_based_config) =
                // Look for flox/flox.toml in $XDG_CONFIG_HOME and then $XDG_CONFIG_DIRS
                flox_dirs.find_config_file(FLOX_CONFIG_FILE)
            {
                debug!(file=?existing_xdg_based_config, "found existing config file");
                existing_xdg_based_config
                    .parent()
                    .expect("filename is always appended to a directory")
                    .to_path_buf()
            } else {
                debug!("no user config file found");
                // fall back to `XDG_CONFIG_HOME/flox`
                flox_dirs.get_config_home().context("$HOME not set")?
            };

            fs::create_dir_all(&config_dir)
                .context(format!("Could not create config directory: {config_dir:?}"))?;
            let config_dir = config_dir
                .canonicalize()
                .context("Could not canonicalize config directory '{config_dir:?}'")?;

            // Allow subshells to find the same config dir.
            // TODO: decide if its worth modifying the env for this.
            // SAFETY: config initially read when there is no concurrent access to env variables.
            unsafe {
                env::set_var(FLOX_CONFIG_DIR_VAR, &config_dir);
            }
            config_dir
        },
    };

    Ok(config_dir)
}

/// Reads a [HierarchicalConfig] from an optional system config file,
/// a user config file and environment variables.
fn raw_config_from_parts(
    flox_dirs: &BaseDirectories,
    user_config_dir: &Path,
    system_config_dir: Option<&Path>,
    env: impl IntoIterator<Item = (String, String)>,
) -> Result<HierarchicalConfig> {
    let cache_dir = flox_dirs.get_cache_home().context("$HOME not set")?;
    let data_dir = flox_dirs.get_data_home().context("$HOME not set")?;
    let state_dir = flox_dirs.get_state_home().context("$HOME not set")?;

    let config_dir = user_config_dir;

    let mut builder = HierarchicalConfig::builder()
        .set_default("default_substituter", "https://cache.flox.dev/")?
        .set_default("cache_dir", cache_dir.to_str().unwrap())?
        .set_default("data_dir", data_dir.to_str().unwrap())?
        .set_default("state_dir", state_dir.to_str().unwrap())?
        // Config dir is added to the config for completeness;
        // the config file cannot change the config dir.
        .set_override("config_dir", config_dir.to_str().unwrap())?;
    // Read System Config first
    if let Some(system_config_dir) = system_config_dir {
        builder = builder.add_source(
            config::File::from(system_config_dir.join(FLOX_CONFIG_FILE))
                .format(config::FileFormat::Toml)
                .required(false),
        );
    };

    // Read User Config
    builder = builder.add_source(
        config::File::from(config_dir.join(FLOX_CONFIG_FILE))
            .format(config::FileFormat::Toml)
            .required(false),
    );

    // Override via env variables
    // Split off FLOX_ prefix, parse nested configs.
    let builder = {
        let mut flox_envs = env
            .into_iter()
            .filter_map(|(k, v)| k.strip_prefix("FLOX_").map(|k| (k.to_owned(), v)))
            .collect::<Vec<_>>();
        builder
            .add_source(mk_environment(&mut flox_envs, "NIX"))
            .add_source(mk_environment(&mut flox_envs, "GITHUB"))
            .add_source(mk_environment(&mut flox_envs, "FEATURES"))
            .add_source(
                Environment::default()
                    .source(Some(HashMap::from_iter(flox_envs)))
                    .try_parsing(true),
            )
    };

    let final_config = builder.build()?;
    Ok(final_config)
}

/// Peel one level of nesting off the `prefix` section of the environment.
///
/// `envs` holds the environment with the `FLOX_` prefix already stripped.
/// The variables belonging to `prefix` are moved out of `envs`
/// and rewritten from `<PREFIX>_SOMETHING_NESTED` to `<PREFIX>#SOMETHING_NESTED`.
/// The returned source splits on `#` (not `_`), so the config crate parses it,
/// case insensitively, as `{ <prefix>: { something_nested: () } }`
/// extracting one level of nesting, with the leaf key keeping its own underscores
/// (`FLOX_FEATURES_SOME_FLAG` -> `features.some_flag`).
fn mk_environment(envs: &mut Vec<(String, String)>, prefix: &str) -> Environment {
    let (prefixed_envs, flox_envs): (HashMap<String, String>, Vec<(String, String)>) = envs
        .iter()
        .partition_map(|(k, v)| match k.strip_prefix(&format!("{prefix}_")) {
            Some(suffix) => Either::Left((format!("{prefix}#{suffix}"), v.to_owned())),
            None => Either::Right((k.to_owned(), v.to_owned())),
        });
    let environment = Environment::with_prefix(prefix)
        .keep_prefix(true)
        .separator("#")
        .source(Some(prefixed_envs))
        .try_parsing(true);
    *envs = flox_envs;
    environment
}

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use indoc::formatdoc;
    use toml_edit::Key;

    use super::*;
    use crate::config::TokenStorageMode;

    // TODO: update the `xdg` crate and build `BaseDirectories` by hand with known (test) paths
    fn mock_flox_dirs() -> BaseDirectories {
        BaseDirectories::new()
    }

    #[test]
    fn test_set_by_env() {
        let tempdir = tempfile::tempdir().unwrap();
        temp_env::with_vars(
            [
                (
                    "HOME",
                    Some(tempdir.path().as_os_str().to_string_lossy().as_ref()),
                ),
                ("FLOX_FLOXHUB_URL", Some("https://example.com")),
            ],
            || {
                // SAFETY: env already isolated from concurrent access via `temp_env`
                unsafe {
                    env::set_var("FLOX_FLOXHUB_URL", "https://example.com");
                }
                let config = Config::parse().unwrap();
                assert_eq!(
                    config
                        .get_verbatim(&Key::parse("floxhub_url").unwrap())
                        .unwrap(),
                    "\"https://example.com/\"".to_string()
                );
                unsafe { env::remove_var(FLOX_CONFIG_DIR_VAR) };
            },
        );
    }

    #[test]
    fn set_by_system_config() {
        let user_config_dir = tempfile::tempdir().unwrap();
        let system_config_dir = tempfile::tempdir().unwrap();

        let floxhub_url = "https://testhub.flox.dev";
        let disable_metrics = true;
        let search_limit = NonZero::new(42).unwrap();

        fs::write(
            system_config_dir.path().join(FLOX_CONFIG_FILE),
            formatdoc! {"
            floxhub_url = '{floxhub_url}'
            disable_metrics = {disable_metrics}
            search_limit = {search_limit}
        "},
        )
        .unwrap();

        // todo: fix mocks to avoid pulling in user config
        fs::write(user_config_dir.path().join(FLOX_CONFIG_FILE), "").unwrap();

        let config = Config::parse_with(
            &mock_flox_dirs(),
            user_config_dir.path(),
            Some(system_config_dir.path()),
            [],
        )
        .unwrap();

        assert_eq!(config.flox.floxhub_url, Some(floxhub_url.parse().unwrap()));
        assert!(config.flox.disable_metrics);
        assert_eq!(config.flox.search_limit, Some(search_limit));
    }

    #[test]
    fn floxhub_token_storage_parses_and_defaults() {
        let user_config_dir = tempfile::tempdir().unwrap();
        let system_config_dir = tempfile::tempdir().unwrap();
        fs::write(system_config_dir.path().join(FLOX_CONFIG_FILE), "").unwrap();

        // Absent → defaults to keyring.
        fs::write(user_config_dir.path().join(FLOX_CONFIG_FILE), "").unwrap();
        let config = Config::parse_with(
            &mock_flox_dirs(),
            user_config_dir.path(),
            Some(system_config_dir.path()),
            [],
        )
        .unwrap();
        assert_eq!(config.flox.floxhub_token_storage, TokenStorageMode::Keyring);

        // Explicit plaintext → parsed.
        fs::write(
            user_config_dir.path().join(FLOX_CONFIG_FILE),
            "floxhub_token_storage = \"plaintext\"\n",
        )
        .unwrap();
        let config = Config::parse_with(
            &mock_flox_dirs(),
            user_config_dir.path(),
            Some(system_config_dir.path()),
            [],
        )
        .unwrap();
        assert_eq!(
            config.flox.floxhub_token_storage,
            TokenStorageMode::Plaintext
        );
    }

    #[test]
    fn user_config_overrides_system() {
        let user_config_dir = tempfile::tempdir().unwrap();
        let system_config_dir = tempfile::tempdir().unwrap();

        let floxhub_url = "https://testhub.flox.dev";
        let disable_metrics = true;
        let search_limit = NonZero::new(42).unwrap();

        fs::write(user_config_dir.path().join(FLOX_CONFIG_FILE), formatdoc! {"
            floxhub_url = '{floxhub_url}'
            disable_metrics = {disable_metrics}
            search_limit = {search_limit}
        "})
        .unwrap();

        fs::write(
            system_config_dir.path().join(FLOX_CONFIG_FILE),
            formatdoc! {"
            floxhub_url = 'https://system.flox.dev'
            disable_metrics = false
            search_limit = 24
        "},
        )
        .unwrap();

        let config = Config::parse_with(
            &mock_flox_dirs(),
            user_config_dir.path(),
            Some(system_config_dir.path()),
            [],
        )
        .unwrap();

        assert_eq!(config.flox.floxhub_url, Some(floxhub_url.parse().unwrap()));
        assert!(config.flox.disable_metrics);
        assert_eq!(config.flox.search_limit, Some(search_limit));
    }

    #[test]
    fn env_overrides_user_and_system() {
        let user_config_dir = tempfile::tempdir().unwrap();
        let system_config_dir = tempfile::tempdir().unwrap();

        let floxhub_url = "https://env.flox.dev";
        let disable_metrics = true;
        let search_limit = NonZero::new(42).unwrap();

        fs::write(user_config_dir.path().join(FLOX_CONFIG_FILE), formatdoc! {"
            floxhub_url = 'https://user.flox.dev'
            disable_metrics = false
            search_limit = 12
        "})
        .unwrap();

        fs::write(
            system_config_dir.path().join(FLOX_CONFIG_FILE),
            formatdoc! {"
            floxhub_url = 'https://system.flox.dev'
            disable_metrics = false
            search_limit = 24
        "},
        )
        .unwrap();

        let env = [
            ("FLOX_FLOXHUB_URL".into(), floxhub_url.to_owned()),
            ("FLOX_DISABLE_METRICS".into(), format!("{disable_metrics}")),
            ("FLOX_SEARCH_LIMIT".into(), format!("{search_limit}")),
        ];

        let config = Config::parse_with(
            &mock_flox_dirs(),
            user_config_dir.path(),
            Some(system_config_dir.path()),
            env,
        )
        .unwrap();

        assert_eq!(config.flox.floxhub_url, Some(floxhub_url.parse().unwrap()));
        assert!(config.flox.disable_metrics);
        assert_eq!(config.flox.search_limit, Some(search_limit));
    }
}
