use std::io;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_config::{Config, FLOX_CONFIG_FILE, ReadWriteError};
use flox_core::vars::do_not_track_from_env;
use flox_events::{EventsBuffer, EventsHub};
use flox_rust_sdk::flox::Flox;
use fslock::LockFile;
use indoc::indoc;
use serde::Serialize;
use serde_json::Value;
use tokio::fs;
use toml_edit::{Key, TomlError};
use tracing::{debug, instrument};

use crate::utils::detached::{SEND_TELEMETRY_LOG_NAME, send_telemetry_log_dir};
use crate::utils::init::{
    METRICS_EVENTS_FILE_NAME,
    METRICS_LOCK_FILE_NAME,
    METRICS_UUID_FILE_NAME,
};
use crate::utils::message;

// Delete the telemetry device ID, the unsent telemetry events and the
// send-telemetry log
#[derive(Bpaf, Clone)]
pub struct ResetMetrics {}
impl ResetMetrics {
    #[instrument(name = "reset-metrics", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        // An event recorded from here on would carry the ID this command
        // deletes, and would recreate the v2 buffer.
        EventsHub::global().clear_client();

        let data_dir = flox.data_dir.clone();
        let cache_dir = flox.cache_dir.clone();
        tokio::task::spawn_blocking(move || delete_device_id_and_buffers(&data_dir, &cache_dir))
            .await??;

        let notice = indoc! {"
            Deleted your telemetry device ID, if one existed.
            Deleted your unsent telemetry events and the send-telemetry log.
            The reset does not delete events that Flox has already received.
            Flox creates a new device ID the next time it runs with telemetry on
            and shows the telemetry notice.

            To turn telemetry off, do one of the following:

              add 'export FLOX_DISABLE_METRICS=true' to your shell profile
              add 'export DO_NOT_TRACK=true' to your shell profile
              run 'flox config --set disable_metrics true'
        "};

        message::plain(notice);
        Ok(())
    }
}

/// Delete the device ID, the two event buffers and the send-telemetry log,
/// which all hold the ID, and the buffers' lock files.
///
/// Holds the metrics lock throughout, so no process creates a new ID or
/// appends to the legacy buffer until the old ID is gone. Deletes the device
/// ID last, so when an earlier deletion fails, the old ID and the events
/// recorded under it stay together and rerunning the command finishes the
/// reset.
fn delete_device_id_and_buffers(data_dir: &Path, cache_dir: &Path) -> Result<()> {
    let metrics_lock_path = cache_dir.join(METRICS_LOCK_FILE_NAME);
    let mut metrics_lock =
        LockFile::open(&metrics_lock_path).context("Could not open metrics lock file")?;
    metrics_lock
        .lock()
        .context("Could not lock metrics lock file")?;

    remove_file_if_present(&cache_dir.join(METRICS_EVENTS_FILE_NAME))?;
    EventsBuffer::delete(data_dir)?;
    remove_file_if_present(&send_telemetry_log_dir(cache_dir).join(SEND_TELEMETRY_LOG_NAME))?;
    remove_file_if_present(&data_dir.join(METRICS_UUID_FILE_NAME))?;

    // Best effort: the lock file holds no data. Deleting it while it is held
    // has the effect described on `EventsBuffer::delete`: a process already
    // waiting on it and a process that starts later do not exclude each other.
    if let Err(err) = remove_file_if_present(&metrics_lock_path) {
        debug!(error = %err, "Failed to delete metrics lock file");
    }
    Ok(())
}

fn remove_file_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result.with_context(|| format!("Could not delete {}", path.display())),
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
        match self {
            ConfigArgs::List => println!("{}", config.get_verbatim(&[])?),
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

                let enables_metrics =
                    key == "disable_metrics" && parsed_value == Value::Bool(false);
                update_config(&flox.config_dir, key, Some(parsed_value))?;

                if enables_metrics && do_not_track_from_env() {
                    message::info(indoc! {"
                        DO_NOT_TRACK still disables telemetry.
                        Flox ignores 'disable_metrics = false' while DO_NOT_TRACK is set.
                    "});
                }
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
pub(crate) fn update_config<V: Serialize>(
    config_dir: &Path,
    key: impl AsRef<str>,
    value: Option<V>,
) -> Result<()> {
    let query = parse_toml_key(key.as_ref()).context("Could not parse key")?;
    update_config_with_query(config_dir, &query, value)
}

/// Like [`update_config`], but takes an already-parsed TOML key path instead of
/// a dot-separated string.
///
/// Use this when a key segment can itself contain `.` — e.g. a filesystem path
/// — which [`parse_toml_key`]'s dot-splitting would otherwise shatter into
/// several nested-table segments.
pub(super) fn update_config_with_query<V: Serialize>(
    config_dir: &Path,
    query: &[Key],
    value: Option<V>,
) -> Result<()> {
    let config_file_path = config_dir.join(FLOX_CONFIG_FILE);

    match Config::write_to_in(config_file_path, query, value) {
                err @ Err(ReadWriteError::ReadConfig(_)) => err.context("Could not read current config file.\nPlease verify the format or reset using `flox config --reset`")?,
                err @ Err(_) => err?,
                Ok(()) => ()
            }
    Ok(())
}

/// Like [`update_config_with_query`], but removes the key and reports whether
/// there was anything to remove.
///
/// Absence is `Ok(false)`, not an error. [`Config::write_to`]'s removal branch
/// raises `ReadWriteError::NotAUserValue` exactly when the key is missing —
/// that is the only place in `flox-config` which constructs the variant — so
/// mapping it to `false` cannot mask an unrelated failure. Genuine failures,
/// including an unreadable or malformed config, still propagate.
pub(super) fn remove_config_key_with_query(config_dir: &Path, query: &[Key]) -> Result<bool> {
    let config_file_path = config_dir.join(FLOX_CONFIG_FILE);

    match Config::write_to_in(config_file_path, query, None::<()>) {
        Ok(()) => Ok(true),
        Err(ReadWriteError::NotAUserValue(_)) => Ok(false),
        Err(err @ ReadWriteError::ReadConfig(_)) => Err(err).context(
            "Could not read current config file.\nPlease verify the format or reset using `flox config --reset`",
        ),
        Err(err) => Err(err.into()),
    }
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

    fn sorted_entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn delete_device_id_and_buffers_leaves_no_telemetry_files() {
        let data_dir = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let log_dir = send_telemetry_log_dir(cache_dir.path());
        std::fs::create_dir_all(&log_dir).unwrap();
        drop(EventsBuffer::read(data_dir.path()).unwrap());
        for path in [
            data_dir.path().join(METRICS_UUID_FILE_NAME),
            cache_dir.path().join(METRICS_EVENTS_FILE_NAME),
            cache_dir.path().join(METRICS_LOCK_FILE_NAME),
            log_dir.join(SEND_TELEMETRY_LOG_NAME),
        ] {
            std::fs::write(path, "").unwrap();
        }

        delete_device_id_and_buffers(data_dir.path(), cache_dir.path()).unwrap();

        assert_eq!(sorted_entries(data_dir.path()), Vec::<String>::new());
        assert_eq!(sorted_entries(cache_dir.path()), vec!["log"]);
        assert_eq!(sorted_entries(&log_dir), Vec::<String>::new());
    }

    #[test]
    fn delete_device_id_and_buffers_keeps_the_device_id_when_a_deletion_fails() {
        let data_dir = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        // A directory in place of the v2 buffer cannot be deleted as a file.
        std::fs::create_dir(data_dir.path().join(flox_events::EVENTS_BUFFER_FILE_NAME)).unwrap();
        std::fs::write(data_dir.path().join(METRICS_UUID_FILE_NAME), "").unwrap();

        delete_device_id_and_buffers(data_dir.path(), cache_dir.path()).unwrap_err();

        assert!(data_dir.path().join(METRICS_UUID_FILE_NAME).exists());
    }
}
