use std::fs;
use std::path::Path;

use flox_core::{WriteError, write_atomically};
use itertools::Itertools;
use serde::Serialize;
use thiserror::Error;
use toml_edit::{DocumentMut, Item, Key, Table, TableLike};
use tracing::{debug, trace};

use crate::config::Config;

/// Error returned by [`Config::get()`]
#[derive(Debug, Error)]
pub enum ReadWriteError {
    #[error("Invalid config key: '{}'",
         _0.iter()
         .map(|key| key.display_repr()
         .into_owned())
         .collect_vec()
         .join("."))]
    InvalidKey(Vec<Key>),
    #[error("Config key '{}' not in user configuration", _0.iter().map(|key| key.display_repr().into_owned()).collect_vec().join("."))]
    NotAUserValue(Vec<Key>),
    #[error(transparent)]
    TomlEdit(#[from] toml_edit::TomlError),
    #[error(transparent)]
    TomlSer(#[from] toml_edit::ser::Error),
    #[error(transparent)]
    TomlDe(#[from] toml_edit::de::Error),
    #[error("Could not read config file: {0}")]
    ReadConfig(std::io::Error),
    #[error("Could not write config file")]
    WriteConfig(#[source] WriteError),
}

/// get a value from the config
///
/// **intended for human consumption/introspection of config only**
///
/// Values in the context should be read from the [Config] type instead!
pub(crate) fn get(config: &Config, path: &[Key]) -> Result<String, ReadWriteError> {
    let document: toml_edit::DocumentMut = toml_edit::ser::to_document(config)?;

    if path.is_empty() {
        return Ok(document.to_string());
    }

    let mut cfg = document.as_table() as &dyn TableLike;

    let (key, parents) = path.split_last().unwrap();

    for (n, segment) in parents.iter().enumerate() {
        let maybe_value = cfg.get(segment).and_then(|item| item.as_table_like());

        match maybe_value {
            Some(v) => cfg = v,
            None => {
                Err(ReadWriteError::InvalidKey(path[..=n].to_vec()))?;
            },
        }
    }

    let value = cfg
        .get(key.as_ref())
        .ok_or(ReadWriteError::InvalidKey(path.to_vec()))?;

    Ok(value.to_string())
}

/// Append or update a key value paring in the toml representation of a partial config
///
/// Validate using [Config]
pub(crate) fn write_to<V: Serialize>(
    config_file: Option<String>,
    path: &[Key],
    value: Option<V>,
) -> Result<String, ReadWriteError> {
    let mut validation_document = toml_edit::ser::to_document(&Config::default())?;

    let mut document = match config_file {
        Some(content) => content.parse::<DocumentMut>()?,
        None => DocumentMut::new(),
    };

    let (mut handle, mut validation) =
        (document.as_table_mut(), validation_document.as_table_mut());

    let (key, parents) = path.split_last().unwrap();

    for segment in parents {
        trace!("stepping into path segment {}", segment);

        if !handle.contains_table(segment) {
            handle.insert(segment, Item::Table(Table::new()));
        }
        if !validation.contains_table(segment) {
            validation.insert(segment, Item::Table(Table::new()));
        }

        handle = handle.get_mut(segment).unwrap().as_table_mut().unwrap();
        validation = validation.get_mut(segment).unwrap().as_table_mut().unwrap();
    }

    trace!("write value for key '{}'", key.display_repr());

    match value {
        None => {
            let _ = handle
                .remove(key.as_ref())
                .ok_or(ReadWriteError::NotAUserValue(path.to_vec()))?;
        },
        Some(ref value) => {
            for handle in [handle, validation] {
                handle.insert(
                    key.as_ref(),
                    Item::Value(value.serialize(toml_edit::ser::ValueSerializer::default())?),
                );
            }
            trace!("try parsing the new virtual config (validation)");
            let validation_config: Config = toml_edit::de::from_document(validation_document)?;

            validation_config.get_verbatim(path)?;
        },
    }

    Ok(document.to_string())
}

pub(crate) fn write_to_in<V: Serialize>(
    config_file_path: impl AsRef<Path>,
    query: &[Key],
    value: Option<V>,
) -> Result<(), ReadWriteError> {
    let config_file_contents = match fs::read_to_string(&config_file_path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                "No existing user config file found in {:?}, creating it now",
                config_file_path.as_ref()
            );
            Ok(None)
        },
        Err(e) => Err(e),
    }
    .map_err(ReadWriteError::ReadConfig)?;

    let config_file_contents = write_to(config_file_contents, query, value)?;

    write_atomically(&config_file_path, config_file_contents)
        .map_err(ReadWriteError::WriteConfig)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;
    use crate::config::AutoActivationPreference;

    #[test]
    fn test_read_bool() {
        let mut config = Config::default();
        config.flox.disable_metrics = true;
        assert_eq!(
            config
                .get_verbatim(&Key::parse("disable_metrics").unwrap())
                .unwrap(),
            "true".to_string()
        );
    }

    #[test]
    fn test_writing_value() {
        let config_content = Config::write_to(
            None,
            &Key::parse("floxhub_url").unwrap(),
            Some("https://example.com"),
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
            floxhub_url = \"https://example.com\"
            "})
    }
    #[test]
    fn test_appending_value() {
        let config_before = indoc! {"
        floxhub_url = \"hello\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("disable_metrics").unwrap(),
            Some(true),
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
        floxhub_url = \"hello\"
        disable_metrics = true
        "});
    }

    #[test]
    fn test_appending_value_keep_comment() {
        let config_before = indoc! {"
        # my FloxHub url is friendly, see:
        floxhub_url = \"hello\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("disable_metrics").unwrap(),
            Some(true),
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
        # my FloxHub url is friendly, see:
        floxhub_url = \"hello\"
        disable_metrics = true
        "});
    }

    #[test]
    fn test_writing_bool() {
        let config_content =
            Config::write_to(None, &Key::parse("disable_metrics").unwrap(), Some(true)).unwrap();
        assert_eq!(config_content, indoc! {"
        disable_metrics = true
        "});
    }

    #[test]
    fn test_writing_invalid() {
        let config_content =
            Config::write_to(None, &Key::parse("does_not_exist").unwrap(), Some("true"));
        assert!(matches!(config_content, Err(ReadWriteError::InvalidKey(_))));
    }

    #[test]
    fn writing_auto_activate_preference_for_path_with_dot() {
        // Regression: an auto-activation preference is keyed by a filesystem
        // path, which can contain `.` (macOS temp dirs live under paths like
        // `/var/folders/...`, and project directories may be named `my.app`).
        // The path must be written as a single literal TOML key rather than a
        // dot-separated key string, which would shatter it into nested tables
        // and fail validation with "unknown variant". `write_to` validates the
        // result by deserializing it, so a successful call already proves the
        // path round-trips back into the typed config.
        let path = "/var/folders/ab/cd.ef/my.project";
        let query = [
            Key::new("auto_activate_environments"),
            Key::new(path.to_string()),
        ];
        let rendered =
            Config::write_to(None, &query, Some(AutoActivationPreference::Deny)).unwrap();
        assert_eq!(rendered, indoc! {r#"
            [auto_activate_environments]
            "/var/folders/ab/cd.ef/my.project" = "deny"
        "#});
    }

    #[test]
    fn test_remove() {
        let config_before = indoc! {"
        # my git base url is friendly, see:
        git_base_url = \"hello\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("git_base_url").unwrap(),
            None::<()>,
        )
        .unwrap();
        assert_eq!(config_content, indoc! {""});
    }

    #[test]
    fn test_remove_invalid() {
        let config_before = indoc! {"
        # my git base url is friendly, see:
        git_base_url = \"hello\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("invalid").unwrap(),
            None::<()>,
        );
        assert!(matches!(
            config_content,
            Err(ReadWriteError::NotAUserValue(_))
        ));
    }

    #[test]
    fn test_remove_not_present() {
        let config_before = indoc! {""};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("git_base_url").unwrap(),
            None::<()>,
        );
        assert!(matches!(
            config_content,
            Err(ReadWriteError::NotAUserValue(_))
        ));
    }

    #[test]
    fn test_remove_nested_not_present() {
        let config_before = indoc! {""};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("trusted_environments.\"foo/bar\"").unwrap(),
            None::<()>,
        );
        assert!(matches!(
            config_content,
            Err(ReadWriteError::NotAUserValue(_))
        ));
    }

    #[test]
    fn test_remove_nested() {
        let config_before = indoc! {"
        [trusted_environments]
        \"foo/bar\" = \"baz\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("trusted_environments.\"foo/bar\"").unwrap(),
            None::<()>,
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
        [trusted_environments]
        "});
    }
}
