use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, Result};
use config::{Config as HierarchicalConfig, Environment};
use flox_core::vars::{DO_NOT_TRACK_VAR, do_not_track};
use itertools::{Either, Itertools};
use serde::de::DeserializeOwned;
use tracing::debug;
use xdg::BaseDirectories;

use crate::{FLOX_CONFIG_FILE, FLOX_DIR_NAME};

const FLOX_CONFIG_DIR_VAR: &str = "FLOX_CONFIG_DIR";

/// The config key that [DO_NOT_TRACK_VAR] forces to `true`.
const DISABLE_METRICS_KEY: &str = "disable_metrics";

/// Creates a configuration from the environment and config files.
///
/// `T` names the keys that are read:
/// a key it does not have is passed over, whatever its value.
///
/// The user config dir is created if it is missing,
/// and exported as `$FLOX_CONFIG_DIR` unless that variable named it.
/// [parse_from] does neither.
pub fn parse<T: DeserializeOwned>() -> Result<T> {
    let env = env::vars().collect::<Vec<_>>();
    let base_directories = BaseDirectories::with_prefix(FLOX_DIR_NAME);
    let user_config_dir =
        prepare_user_config_dir(locate_user_config_dir(&base_directories, &env)?)?;
    parse_with(
        &base_directories,
        &user_config_dir,
        locate_system_config_dir(&env)?.as_deref(),
        env,
    )
}

/// Like [parse], for the environment `env`, and without changing anything:
/// no directory is created and no variable is set.
/// The process environment is not read.
pub fn parse_from<T: DeserializeOwned>(
    env: impl IntoIterator<Item = (String, String)>,
) -> Result<T> {
    let env = env.into_iter().collect::<Vec<_>>();
    let base_directories = base_directories_from(&env);
    let user_config_dir = user_config_dir_from(&env)?;
    parse_with(
        &base_directories,
        &user_config_dir,
        locate_system_config_dir(&env)?.as_deref(),
        env,
    )
}

/// The user config dir [parse_from] reads for the environment `env`,
/// located as [parse] locates it, and not created.
///
/// [parse] would create it if it is missing, and fails where it cannot;
/// a caller that reads without changing anything can tell from this path
/// whether [parse] would.
pub fn user_config_dir_from(env: &[(String, String)]) -> Result<PathBuf> {
    let base_directories = base_directories_from(env);
    Ok(locate_user_config_dir(&base_directories, env)?.into_path())
}

/// Like [parse], with explicit directories and environment.
pub fn parse_with<T: DeserializeOwned>(
    flox_dirs: &BaseDirectories,
    user_config_dir: &Path,
    system_config_dir: Option<&Path>,
    env: impl IntoIterator<Item = (String, String)>,
) -> Result<T> {
    let env = env.into_iter().collect::<Vec<_>>();
    let tracking_refused = env
        .iter()
        .any(|(key, value)| key == DO_NOT_TRACK_VAR && do_not_track(value));
    let final_config = raw_config_from_parts(flox_dirs, user_config_dir, system_config_dir, env)?;

    let cli_config: T = final_config
        .to_owned()
        .try_deserialize()
        .context("Could not parse config")?;
    if !tracking_refused {
        return Ok(cli_config);
    }
    // `DO_NOT_TRACK` wins over `disable_metrics = false` from every source,
    // including `FLOX_DISABLE_METRICS=false`: the NixOS module, for example,
    // exports that `false` to every Flox service by default.
    // It is set on the merged sources rather than on a field of `T`,
    // after they parsed without it:
    // a value that cannot be read stays an error whatever `DO_NOT_TRACK` holds.
    HierarchicalConfig::builder()
        .add_source(final_config)
        .set_override(DISABLE_METRICS_KEY, true)?
        .build()?
        .try_deserialize()
        .context("Could not parse config")
}

/// The value of the variable `name` in `env`.
fn var<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
    env.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// flox's base directories as `env` names them.
///
/// These are the directories [BaseDirectories::with_prefix] reads from the
/// process environment, by the same rules:
/// a relative path counts as unset, and the defaults are under `$HOME`.
/// Without a non-empty `HOME` in `env` they are unset;
/// the user database is not asked.
pub fn base_directories_from(env: &[(String, String)]) -> BaseDirectories {
    let absolute = |name: &str| {
        var(env, name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let absolutes = |name: &str| {
        var(env, name)
            .map(|paths| {
                env::split_paths(paths)
                    .filter(|path| path.is_absolute())
                    .collect::<Vec<_>>()
            })
            .filter(|paths| !paths.is_empty())
    };
    let home = var(env, "HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from);
    let in_home = |dir: &str| home.as_ref().map(|home| home.join(dir));

    // `BaseDirectories` is non-exhaustive and has no constructor that takes
    // an environment, so every directory of one made from the process is
    // replaced.
    let mut dirs = BaseDirectories::with_prefix(FLOX_DIR_NAME);
    dirs.data_home = absolute("XDG_DATA_HOME").or_else(|| in_home(".local/share"));
    dirs.config_home = absolute("XDG_CONFIG_HOME").or_else(|| in_home(".config"));
    dirs.cache_home = absolute("XDG_CACHE_HOME").or_else(|| in_home(".cache"));
    dirs.state_home = absolute("XDG_STATE_HOME").or_else(|| in_home(".local/state"));
    dirs.data_dirs = absolutes("XDG_DATA_DIRS")
        .unwrap_or_else(|| vec!["/usr/local/share".into(), "/usr/share".into()]);
    dirs.config_dirs = absolutes("XDG_CONFIG_DIRS").unwrap_or_else(|| vec!["/etc/xdg".into()]);
    dirs.runtime_dir = absolute("XDG_RUNTIME_DIR");
    dirs
}

/// Locates the system wide flox config dir.
///
/// By default that is `/etc`.
/// The directory can be overridden by the user via the environment variable
/// `$FLOX_SYSTEM_CONFIG_DIR`.
///
/// IFF `$FLOX_SYSTEM_CONFIG_DIR` is set to an empty string, system config will be ignored.
fn locate_system_config_dir(env: &[(String, String)]) -> Result<Option<PathBuf>> {
    // TODO: make constant
    match var(env, "FLOX_SYSTEM_CONFIG_DIR") {
        Some("") => Ok(None),
        Some(path) => Ok(Some(path.into())),
        None => Ok(Some(PathBuf::from("/etc"))),
    }
}

/// The user specific config dir, by where [locate_user_config_dir] found it.
#[derive(Clone, Debug, PartialEq)]
enum UserConfigDir {
    /// Named by a non-empty `$FLOX_CONFIG_DIR`.
    Overridden(PathBuf),
    /// Found in, or defaulted to, the xdg dirs.
    Xdg(PathBuf),
}

impl UserConfigDir {
    fn into_path(self) -> PathBuf {
        match self {
            UserConfigDir::Overridden(path) | UserConfigDir::Xdg(path) => path,
        }
    }
}

/// Locates the user specific config dir, without creating it.
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
fn locate_user_config_dir(
    flox_dirs: &BaseDirectories,
    env: &[(String, String)],
) -> Result<UserConfigDir> {
    let user_config_dir = var(env, FLOX_CONFIG_DIR_VAR);

    let config_dir = match user_config_dir {
        Some(path) if !path.is_empty() => {
            debug!(?path, "Global config directory overridden");
            UserConfigDir::Overridden(path.into())
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
            UserConfigDir::Xdg(config_dir)
        },
    };

    Ok(config_dir)
}

/// Creates the user specific config dir if it is missing.
///
/// One found in the xdg dirs is also canonicalized
/// and exported as `$FLOX_CONFIG_DIR`.
fn prepare_user_config_dir(located: UserConfigDir) -> Result<PathBuf> {
    let config_dir = match located {
        UserConfigDir::Overridden(path) => {
            fs::create_dir_all(&path)
                .context(format!("Could not create config directory: {path:?}"))?;
            path
        },
        UserConfigDir::Xdg(config_dir) => {
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
    use crate::config::{Config, TokenStorageMode};

    /// Base directories under a home that does not exist,
    /// whatever the environment of the test process holds.
    fn mock_flox_dirs() -> BaseDirectories {
        base_directories_from(&env_of(&[("HOME", "/home/user")]))
    }

    #[test]
    fn base_directories_follow_the_given_environment() {
        let dirs = base_directories_from(&env_of(&[
            ("HOME", "/home/user"),
            ("XDG_DATA_HOME", "/data"),
            ("XDG_CACHE_HOME", "relative"),
            ("XDG_CONFIG_DIRS", "/one::relative:/two"),
        ]));

        assert_eq!(dirs.get_data_home(), Some(PathBuf::from("/data/flox")));
        assert_eq!(
            dirs.get_config_home(),
            Some(PathBuf::from("/home/user/.config/flox"))
        );
        assert_eq!(
            dirs.get_cache_home(),
            Some(PathBuf::from("/home/user/.cache/flox"))
        );
        assert_eq!(
            dirs.get_state_home(),
            Some(PathBuf::from("/home/user/.local/state/flox"))
        );
        assert_eq!(dirs.get_config_dirs(), [
            PathBuf::from("/one/flox"),
            PathBuf::from("/two/flox")
        ]);
    }

    #[test]
    fn base_directories_are_what_the_process_would_get() {
        let tempdir = tempfile::tempdir().unwrap();
        let home = tempdir.path().to_str().unwrap();
        let vars = [
            ("HOME", Some(home)),
            ("XDG_DATA_HOME", Some("/data")),
            ("XDG_CONFIG_HOME", None),
            ("XDG_CACHE_HOME", Some("relative")),
            ("XDG_STATE_HOME", None),
            ("XDG_DATA_DIRS", None),
            ("XDG_CONFIG_DIRS", Some("/one::relative:/two")),
            ("XDG_RUNTIME_DIR", None),
        ];
        let env = vars
            .iter()
            .filter_map(|(key, value)| Some((key.to_string(), (*value)?.to_string())))
            .collect::<Vec<_>>();

        let from_process =
            temp_env::with_vars(vars, || BaseDirectories::with_prefix(FLOX_DIR_NAME));
        let from_env = base_directories_from(&env);

        assert_eq!(from_env.get_data_home(), from_process.get_data_home());
        assert_eq!(from_env.get_config_home(), from_process.get_config_home());
        assert_eq!(from_env.get_cache_home(), from_process.get_cache_home());
        assert_eq!(from_env.get_state_home(), from_process.get_state_home());
        assert_eq!(from_env.get_data_dirs(), from_process.get_data_dirs());
        assert_eq!(from_env.get_config_dirs(), from_process.get_config_dirs());
        assert_eq!(from_env.runtime_dir, from_process.runtime_dir);
    }

    #[test]
    fn base_directories_without_home_are_unset() {
        let dirs = base_directories_from(&env_of(&[("HOME", "")]));

        assert_eq!(dirs.get_data_home(), None);
        assert_eq!(dirs.get_config_home(), None);
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

    fn env_of(vars: &[(&str, &str)]) -> Vec<(String, String)> {
        vars.iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn system_config_dir_is_located_in_the_given_environment() {
        assert_eq!(
            locate_system_config_dir(&env_of(&[])).unwrap(),
            Some(PathBuf::from("/etc"))
        );
        assert_eq!(
            locate_system_config_dir(&env_of(&[("FLOX_SYSTEM_CONFIG_DIR", "")])).unwrap(),
            None
        );
        assert_eq!(
            locate_system_config_dir(&env_of(&[("FLOX_SYSTEM_CONFIG_DIR", "/system")])).unwrap(),
            Some(PathBuf::from("/system"))
        );
    }

    #[test]
    fn user_config_dir_override_is_read_from_the_given_environment() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_dir = tempdir.path().join("config");
        let env = env_of(&[(FLOX_CONFIG_DIR_VAR, config_dir.to_str().unwrap())]);

        assert_eq!(
            locate_user_config_dir(&mock_flox_dirs(), &env).unwrap(),
            UserConfigDir::Overridden(config_dir.clone())
        );
        assert!(!config_dir.exists(), "locating must not create the dir");

        assert_eq!(
            prepare_user_config_dir(UserConfigDir::Overridden(config_dir.clone())).unwrap(),
            config_dir
        );
        assert!(config_dir.is_dir(), "preparing creates the dir");
    }

    #[test]
    fn user_config_dir_is_located_without_being_created() {
        let tempdir = tempfile::tempdir().unwrap();
        let home = tempdir.path().join("home");
        fs::create_dir(&home).unwrap();
        let env = env_of(&[
            ("HOME", home.to_str().unwrap()),
            ("XDG_CONFIG_DIRS", home.to_str().unwrap()),
        ]);

        let located = user_config_dir_from(&env).unwrap();

        assert_eq!(located, home.join(".config").join(FLOX_DIR_NAME));
        assert!(!located.exists());

        let overridden = tempdir.path().join("overridden");
        let env = env_of(&[
            ("HOME", home.to_str().unwrap()),
            (FLOX_CONFIG_DIR_VAR, overridden.to_str().unwrap()),
        ]);
        assert_eq!(user_config_dir_from(&env).unwrap(), overridden);
        assert!(!overridden.exists());
    }

    #[test]
    fn parse_from_reads_the_files_of_the_given_environment() {
        let tempdir = tempfile::tempdir().unwrap();
        let home = tempdir.path().join("home");
        let system_config_dir = tempdir.path().join("system");
        let user_config_dir = home.join(".config").join(FLOX_DIR_NAME);
        for dir in [&system_config_dir, &user_config_dir] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(
            system_config_dir.join(FLOX_CONFIG_FILE),
            "disable_metrics = true\nsearch_limit = 24\n",
        )
        .unwrap();
        fs::write(
            user_config_dir.join(FLOX_CONFIG_FILE),
            "search_limit = 42\n",
        )
        .unwrap();

        let config: Config = parse_from(env_of(&[
            ("HOME", home.to_str().unwrap()),
            (
                "FLOX_SYSTEM_CONFIG_DIR",
                system_config_dir.to_str().unwrap(),
            ),
            ("FLOX_FLOXHUB_TOKEN_STORAGE", "plaintext"),
        ]))
        .unwrap();

        assert!(config.flox.disable_metrics);
        assert_eq!(config.flox.search_limit, NonZero::new(42));
        assert_eq!(
            config.flox.floxhub_token_storage,
            TokenStorageMode::Plaintext
        );
    }

    #[test]
    fn parse_from_changes_neither_the_disk_nor_the_environment() {
        let tempdir = tempfile::tempdir().unwrap();
        let home = tempdir.path().join("home");
        fs::create_dir(&home).unwrap();

        temp_env::with_var_unset(FLOX_CONFIG_DIR_VAR, || {
            parse_from::<Config>(env_of(&[
                ("HOME", home.to_str().unwrap()),
                ("FLOX_SYSTEM_CONFIG_DIR", ""),
            ]))
            .unwrap();
            assert_eq!(env::var_os(FLOX_CONFIG_DIR_VAR), None);

            let overridden = tempdir.path().join("overridden");
            parse_from::<Config>(env_of(&[
                ("HOME", home.to_str().unwrap()),
                ("FLOX_SYSTEM_CONFIG_DIR", ""),
                (FLOX_CONFIG_DIR_VAR, overridden.to_str().unwrap()),
            ]))
            .unwrap();
            assert!(!overridden.exists());
            assert_eq!(env::var_os(FLOX_CONFIG_DIR_VAR), None);
        });

        assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
    }

    /// Parse `disable_metrics` from a system and a user config file, each
    /// setting it to `files_value`, plus the given environment.
    fn disable_metrics_with(files_value: bool, env: &[(&str, &str)]) -> bool {
        let user_config_dir = tempfile::tempdir().unwrap();
        let system_config_dir = tempfile::tempdir().unwrap();
        for dir in [&user_config_dir, &system_config_dir] {
            fs::write(
                dir.path().join(FLOX_CONFIG_FILE),
                format!("disable_metrics = {files_value}\n"),
            )
            .unwrap();
        }
        let env = env
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()));

        Config::parse_with(
            &mock_flox_dirs(),
            user_config_dir.path(),
            Some(system_config_dir.path()),
            env,
        )
        .unwrap()
        .flox
        .disable_metrics
    }

    #[test]
    fn do_not_track_overrides_disable_metrics_false_in_files_and_env() {
        assert!(disable_metrics_with(false, &[
            ("FLOX_DISABLE_METRICS", "false"),
            ("DO_NOT_TRACK", "1"),
        ]));
    }

    #[test]
    fn do_not_track_values_that_disable_metrics() {
        for value in ["1", "true", "TRUE", "yes", "anything", " 1 ", "2"] {
            assert!(
                disable_metrics_with(false, &[("DO_NOT_TRACK", value)]),
                "DO_NOT_TRACK={value:?} should disable metrics"
            );
        }
    }

    #[test]
    fn do_not_track_values_that_leave_metrics_unchanged() {
        for value in ["", "  ", "0", " 0 ", "false", "FALSE", "False", " false "] {
            assert!(
                !disable_metrics_with(false, &[("DO_NOT_TRACK", value)]),
                "DO_NOT_TRACK={value:?} should not disable metrics"
            );
            assert!(
                disable_metrics_with(true, &[("DO_NOT_TRACK", value)]),
                "DO_NOT_TRACK={value:?} should not enable metrics"
            );
        }
    }
}
