use std::io;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use fslock::LockFile;
use indoc::indoc;
use serde::Serialize;
use serde_json::Value;
use tokio::fs;
use toml_edit::{Key, TomlError};
use tracing::{debug, instrument};

use crate::config::{Config, FLOX_CONFIG_FILE, ReadWriteError};
use crate::subcommand_metric;
use crate::utils::message;
use crate::utils::metrics::{
    METRICS_EVENTS_FILE_NAME,
    METRICS_LOCK_FILE_NAME,
    METRICS_UUID_FILE_NAME,
};

// Reset the metrics queue (if any), reset metrics ID, and re-prompt for consent
#[derive(Bpaf, Clone)]
pub struct ResetMetrics {}
impl ResetMetrics {
    #[instrument(name = "reset-metrics", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("reset-metrics");
        let mut metrics_lock = LockFile::open(&flox.cache_dir.join(METRICS_LOCK_FILE_NAME))?;
        tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;

        if let Err(err) =
            tokio::fs::remove_file(flox.cache_dir.join(METRICS_EVENTS_FILE_NAME)).await
        {
            match err.kind() {
                std::io::ErrorKind::NotFound => {},
                _ => Err(err)?,
            }
        }

        if let Err(err) = tokio::fs::remove_file(flox.data_dir.join(METRICS_UUID_FILE_NAME)).await {
            match err.kind() {
                std::io::ErrorKind::NotFound => {},
                _ => Err(err)?,
            }
        }

        let notice = indoc! {"
            Successfully reset telemetry ID for this machine!

            A new ID will be assigned next time you use Flox.

            The collection of metrics can be disabled in the following ways:

                environment: FLOX_DISABLE_METRICS=true
                user-wide: flox config --set disable_metrics true
                system-wide: update /etc/flox.toml as described in flox-config(1)
        "};

        message::plain(notice);
        Ok(())
    }
}

#[derive(Bpaf, Clone)]
#[bpaf(fallback(ConfigArgs::List))]
pub enum ConfigArgs {
    /// List the current values of all options
    #[bpaf(short, long)]
    List,
    /// Reset all options to their default values without further confirmation
    #[bpaf(short, long)]
    Reset,
    /// Set a config value
    Set(#[bpaf(external(config_set))] ConfigSet),
    /// Delete a config value
    Delete(#[bpaf(external(config_delete))] ConfigDelete),
}

impl ConfigArgs {
    /// handle config flags like commands
    #[instrument(name = "config", skip_all)]
    pub async fn handle(&self, config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("config");
        match self {
            ConfigArgs::List => println!("{}", config.get(&[])?),
            ConfigArgs::Reset => {
                match fs::remove_file(&flox.config_dir.join(FLOX_CONFIG_FILE)).await {
                    Err(err) if err.kind() != io::ErrorKind::NotFound => {
                        Err(err).context("Could not reset config file")?
                    },
                    _ => (),
                }
            },
            ConfigArgs::Set(ConfigSet { key, value, .. }) => {
                let parsed_value = match Value::from_str(value) {
                    Ok(parsed) => {
                        debug!(supplied = value, ?parsed, "parsed config value");
                        parsed
                    },
                    Err(error) => {
                        debug!(
                            supplied = value,
                            ?error,
                            "failed to parse as JSON value, treating as unquoted string"
                        );
                        Value::String(value.clone())
                    },
                };

                update_config(&flox.config_dir, key, Some(parsed_value))?
            },
            ConfigArgs::Delete(ConfigDelete { key, .. }) => {
                update_config::<()>(&flox.config_dir, key, None)?
            },
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Bpaf)]
#[bpaf(adjacent)]
pub struct ConfigSet {
    /// set <key> to <string>
    #[allow(unused)]
    set: (),
    /// Configuration key
    #[bpaf(positional("key"))]
    key: String,
    /// Configuration value (string)
    #[bpaf(positional("string"))]
    value: String,
}

#[derive(Debug, Clone, Bpaf)]
#[allow(unused)]
pub struct ConfigDelete {
    /// Delete config key
    #[bpaf(long("delete"), argument("key"))]
    key: String,
}

/// wrapper around [Config::write_to]
pub(super) fn update_config<V: Serialize>(
    config_dir: &Path,
    key: impl AsRef<str>,
    value: Option<V>,
) -> Result<()> {
    let query = parse_toml_key(key.as_ref()).context("Could not parse key")?;

    let config_file_path = config_dir.join(FLOX_CONFIG_FILE);

    match Config::write_to_in(config_file_path, &query, value) {
                err @ Err(ReadWriteError::ReadConfig(_)) => err.context("Could not read current config file.\nPlease verify the format or reset using `flox config --reset`")?,
                err @ Err(_) => err?,
                Ok(()) => ()
            }
    Ok(())
}

/// Parse a TOML key from a string, quoting any segments where necessary, so
/// that a user doesn't need to understand the intricacies of TOML.
fn parse_toml_key(key: &str) -> Result<Vec<Key>, TomlError> {
    let normalized_key = key
        .split('.')
        .map(|segment| {
            let quoting_not_needed = segment
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
            let contains_some_quotes = segment.contains('"') || segment.contains('\'');

            if quoting_not_needed || contains_some_quotes {
                segment.to_string()
            } else {
                format!("'{}'", segment)
            }
        })
        .collect::<Vec<_>>()
        .join(".");

    Key::parse(&normalized_key)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn parse_toml_key_no_quoting_needed() {
        let key = "trusted_environments.foo.bar";
        let parsed = parse_toml_key(key).unwrap();
        assert_eq!(parsed, vec!["trusted_environments", "foo", "bar"]);
    }

    #[test]
    fn parse_toml_key_adds_quoting() {
        let key = "trusted_environments.foo/bar";
        let parsed = parse_toml_key(key).unwrap();
        assert_eq!(parsed, vec!["trusted_environments", "foo/bar"]);
    }

    #[test]
    fn parse_toml_key_already_single_quoted() {
        let key = "trusted_environments.'foo/bar'";
        let parsed = parse_toml_key(key).unwrap();
        assert_eq!(parsed, vec!["trusted_environments", "foo/bar"]);
    }

    #[test]
    fn parse_toml_key_already_double_quoted() {
        let key = r#"trusted_environments."foo/bar""#;
        let parsed = parse_toml_key(key).unwrap();
        assert_eq!(parsed, vec!["trusted_environments", "foo/bar"]);
    }

    #[test]
    fn parse_toml_key_already_double_quoted_dotted() {
        let key = r#"trusted_environments."foo.bar""#;
        let parsed = parse_toml_key(key).unwrap();
        assert_eq!(parsed, vec!["trusted_environments", "foo.bar"]);
    }

    #[test]
    fn parse_toml_key_stray_single_quote() {
        let key = "trusted_environments.foo'bar";
        let err = parse_toml_key(key).unwrap_err();
        assert_eq!(err.to_string(), indoc! {r#"
            TOML parse error at line 1, column 25
              |
            1 | trusted_environments.foo'bar
              |                         ^
            invalid unquoted key, expected letters, numbers, `-`, `_`
        "#});
    }

    #[test]
    fn parse_toml_key_stray_double_quote() {
        let key = r#"trusted_environments.foo"bar"#;
        let err = parse_toml_key(key).unwrap_err();
        assert_eq!(err.to_string(), indoc! {r#"
            TOML parse error at line 1, column 25
              |
            1 | trusted_environments.foo"bar
              |                         ^
            invalid unquoted key, expected letters, numbers, `-`, `_`
        "#});
    }
}
