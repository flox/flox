use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

use anyhow::{Result, anyhow};
use flox_core::vars::FLOX_SENTRY_ENV;
use flox_rust_sdk::data::FloxVersion;
use flox_rust_sdk::flox::FLOX_VERSION;
use futures::Future;
use indoc::formatdoc;
use reqwest;
use sentry::integrations::anyhow::capture_anyhow;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{Duration, OffsetDateTime};
use tracing::debug;

use crate::config::InstallerChannel;
use crate::utils::dialog::Dialog;
use crate::utils::errors::display_chain;
use crate::utils::{TRAILING_NETWORK_CALL_TIMEOUT, message};

// Relative to flox executable
const DEFAULT_UPDATE_INSTRUCTIONS: &str =
    "Get the latest at https://flox.dev/docs/install-flox/#upgrade-existing-flox-installation";
const UPDATE_INSTRUCTIONS_RELATIVE_FILE_PATH: &str =
    "../../share/flox/files/update-instructions.txt";
const UPDATE_NOTIFICATION_FILE_NAME: &str = "update-check-timestamp.json";
const UPDATE_NOTIFICATION_EXPIRY: Duration = Duration::days(1);

/// Timestamp we serialize to a file to trackwhen we last checked
/// whether an update is available
#[derive(Deserialize, Serialize)]
struct LastUpdateCheck {
    #[serde(with = "time::serde::iso8601")]
    last_update_check: OffsetDateTime,
}

/// [UpdateNotification] stores a version that the user should be notified is
/// available.
///
/// After notifying, `notification_file` should be written with a timestamp to
/// track that the user was notified.
#[derive(Debug, PartialEq)]
pub(crate) struct UpdateNotification {
    /// `new_version` that the user should be notified is available
    ///
    /// It is assumed that it has already been verified that
    /// new_version != FLOX_VERSION
    new_version: String,
    notification_file: PathBuf,
}

#[derive(Debug, Error)]
pub(crate) enum UpdateNotificationError {
    /// If someone can't check for updates because of a network error, we'll
    /// want to silently ignore it.
    #[error("network error")]
    Network(#[source] reqwest::Error),
    /// If someone can't check for updates because of an IO error, we'll want to
    /// silently ignore it.
    #[error("IO error")]
    Io(#[source] io::Error),
    /// Other errors indicate something we didn't expect may have happened,
    /// so we want to report it with Sentry.
    #[error(transparent)]
    WeMayHaveMessedUp(#[from] anyhow::Error),
}

#[derive(Debug, PartialEq)]
pub(crate) enum UpdateCheckResult {
    /// Updates we're not checked.
    /// Either because the check cooldown hasn't expired,
    /// the user is in development mode,
    /// or we can't prompt the user.
    Skipped,
    /// No update available, but the notification file is or expired
    /// Used to cause a refresh of the notification file
    /// and reset
    RefreshNotificationFile(PathBuf),
    /// An update is available
    UpdateAvailable(UpdateNotification),
}

impl UpdateNotification {
    pub async fn check_for_and_print_update_notification(
        cache_dir: impl AsRef<Path>,
        release_channel: &Option<InstallerChannel>,
    ) {
        Self::handle_update_result(
            Self::check_for_update(cache_dir, release_channel).await,
            release_channel,
        )
    }

    /// If the user hasn't been notified of an update after
    /// UPDATE_NOTIFICATION_EXPIRY time has passed, check for an update.
    pub async fn check_for_update(
        cache_dir: impl AsRef<Path>,
        release_channel: &Option<InstallerChannel>,
    ) -> Result<UpdateCheckResult, UpdateNotificationError> {
        let notification_file = cache_dir.as_ref().join(UPDATE_NOTIFICATION_FILE_NAME);
        // Release channel won't be set for development builds.
        // Skip printing an update notification.
        let Some(release_env) = release_channel else {
            debug!("Skipping update check in development mode");
            return Ok(UpdateCheckResult::Skipped);
        };

        if !Dialog::can_prompt() {
            debug!("Skipping update check because we can't prompt the user");
            return Ok(UpdateCheckResult::Skipped);
        }

        Self::check_for_update_inner(
            notification_file,
            &FLOX_VERSION,
            Self::get_latest_version(release_env),
            UPDATE_NOTIFICATION_EXPIRY,
        )
        .await
    }

    /// If the user hasn't been notified of an update after `expiry` time has
    /// passed, check for an update.
    async fn check_for_update_inner(
        notification_file: PathBuf,
        current_version: &FloxVersion,
        get_latest_version_future: impl Future<Output = Result<String, UpdateNotificationError>>,
        expiry: Duration,
    ) -> Result<UpdateCheckResult, UpdateNotificationError> {
        // Return early if we find a notification_file with a last_notification
        // that hasn't expired
        match fs::read_to_string(&notification_file) {
            // If the file doesn't it exist, it means we haven't shown the notification recently
            Err(e) if e.kind() == io::ErrorKind::NotFound => {},
            Ok(contents) => {
                let update_notification: LastUpdateCheck = serde_json::from_str(&contents)
                    .map_err(|e| UpdateNotificationError::WeMayHaveMessedUp(anyhow!(e)))?;

                let now = OffsetDateTime::now_utc();
                if now - update_notification.last_update_check < expiry {
                    return Ok(UpdateCheckResult::Skipped);
                }
            },
            Err(e) => Err(UpdateNotificationError::Io(e))?,
        };

        let new_version_str = get_latest_version_future.await?;
        let Ok(new_version) = new_version_str.parse::<FloxVersion>() else {
            return Err(UpdateNotificationError::WeMayHaveMessedUp(anyhow!(
                "version '{new_version_str}' is invalid."
            )));
        };

        match current_version.partial_cmp(&new_version) {
            None => Ok(UpdateCheckResult::Skipped),
            Some(std::cmp::Ordering::Less) => {
                Ok(UpdateCheckResult::UpdateAvailable(UpdateNotification {
                    new_version: new_version.to_string(),
                    notification_file,
                }))
            },
            // current_version is Equal or Greater than new_version
            _ => Ok(UpdateCheckResult::RefreshNotificationFile(
                notification_file,
            )),
        }
    }

    /// Print if there's a new version available,
    /// or handle an error
    pub fn handle_update_result(
        update_notification: Result<UpdateCheckResult, UpdateNotificationError>,
        release_env: &Option<InstallerChannel>,
    ) {
        match update_notification {
            Ok(UpdateCheckResult::Skipped) => {},
            Ok(UpdateCheckResult::RefreshNotificationFile(notification_file)) => {
                Self::write_notification_file(notification_file);
            },
            Ok(UpdateCheckResult::UpdateAvailable(update_notification)) => {
                Self::write_notification_file(&update_notification.notification_file);
                update_notification.print_new_version_available(release_env);
            },
            Err(UpdateNotificationError::WeMayHaveMessedUp(e)) => {
                debug!("Failed to check for CLI updates. Sending error to Sentry if enabled");
                capture_anyhow(&anyhow!("Failed to check for CLI updates: {e}"));
            },
            Err(e) => {
                debug!(
                    "Failed to check for CLI update. Ignoring error: {}",
                    display_chain(&e)
                );
            },
        }
    }

    // Check for update instructions file which is located relative to the current executable
    // and is created by an installer
    fn update_instructions(
        update_instructions_relative_file_path: &str,
        release_env: &Option<InstallerChannel>,
    ) -> String {
        let instructions: Cow<str> = 'inst: {
            let Ok(exe) = env::current_exe() else {
                break 'inst DEFAULT_UPDATE_INSTRUCTIONS.into();
            };

            let Ok(update_instructions_file) = exe
                .join(update_instructions_relative_file_path)
                .canonicalize()
            else {
                break 'inst DEFAULT_UPDATE_INSTRUCTIONS.into();
            };

            debug!(
                "Looking for update instructions file at: {}",
                update_instructions_file.display()
            );
            break 'inst fs::read_to_string(update_instructions_file)
                .map(|docs| format!("Get the latest with:\n{}", indent::indent_all_by(2, docs)))
                .unwrap_or(DEFAULT_UPDATE_INSTRUCTIONS.to_string())
                .into();
        };

        instructions.replace(
            FLOX_SENTRY_ENV
                .clone()
                .unwrap_or("stable".to_string())
                .as_str(),
            &release_env.clone().unwrap_or_default().to_string(),
        )
    }

    /// If a new version is available, print a message to the user.
    ///
    /// Write the notification_file with the current time.
    fn print_new_version_available(self, release_env: &Option<InstallerChannel>) {
        let release_env_unwrapped = release_env.clone().unwrap_or_default();
        if release_env_unwrapped.to_string()
            == *FLOX_SENTRY_ENV.clone().unwrap_or("stable".to_string())
        {
            message::plain(formatdoc! {"

                🚀  Flox has a new version available. {} -> {}

                {}
            ",
                *FLOX_VERSION,
                self.new_version,
                Self::update_instructions(UPDATE_INSTRUCTIONS_RELATIVE_FILE_PATH,release_env),
            });
        } else {
            message::plain(formatdoc! {"

                🚀  Flox has a new version available on the {} channel. {} -> {}

                Go to https://downloads.flox.dev/?prefix=by-env/{} to download
            ",
                release_env_unwrapped,
                *FLOX_VERSION,
                self.new_version,
                release_env_unwrapped,
            });
        }
    }

    fn write_notification_file(notification_file: impl AsRef<Path>) {
        let last_notification = LastUpdateCheck {
            last_update_check: OffsetDateTime::now_utc(),
        };

        let notification_file_contents = match serde_json::to_string(&last_notification) {
            Ok(contents) => contents,
            Err(e) => {
                debug!("Failed to serialize update notification file: {e}");
                return;
            },
        };

        match fs::write(notification_file, notification_file_contents) {
            Ok(_) => {},
            Err(e) => {
                let e = UpdateNotificationError::Io(e);
                debug!("Failed to write update notification file: {e}");
            },
        }
    }

    /// Get latest version from downloads.flox.dev
    ///
    /// Timeout after TRAILING_NETWORK_CALL_TIMEOUT
    async fn get_latest_version(
        release_env: &InstallerChannel,
    ) -> Result<String, UpdateNotificationError> {
        let client = reqwest::Client::new();

        let request = client
            .get(format!(
                "https://downloads.flox.dev/by-env/{release_env}/LATEST_VERSION",
            ))
            .timeout(TRAILING_NETWORK_CALL_TIMEOUT);

        let response = request.send().await.map_err(|e| {
            // We'll want to ignore errors if network is non-existent or slow
            if e.is_connect() || e.is_timeout() {
                UpdateNotificationError::Network(e)
            } else {
                UpdateNotificationError::WeMayHaveMessedUp(anyhow!(e))
            }
        })?;

        if response.status().is_success() {
            Ok(response
                .text()
                .await
                .map_err(|e| UpdateNotificationError::WeMayHaveMessedUp(anyhow!(e)))?
                .trim()
                .to_string())
        } else {
            Err(UpdateNotificationError::WeMayHaveMessedUp(anyhow!(
                "got response body:\n{}",
                response
                    .text()
                    .await
                    .unwrap_or_else(|e| format!("couldn't decode body: {e}"))
                    .trim()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use sentry::test::with_captured_events;
    use tempfile::tempdir;

    use super::*;

    /// [UpdateNotification::handle_update_result] should write notification_file,
    /// if an update is available
    #[test]
    fn handle_update_result_writes_file_if_update_available() {
        let temp_dir = tempdir().unwrap();
        let notification_file = temp_dir.path().join("notification_file");

        UpdateNotification::handle_update_result(
            Ok(UpdateCheckResult::UpdateAvailable(UpdateNotification {
                new_version: "new_version".to_string(),
                notification_file: notification_file.clone(),
            })),
            &Some(InstallerChannel::Stable),
        );

        serde_json::from_str::<LastUpdateCheck>(&fs::read_to_string(notification_file).unwrap())
            .unwrap();
    }

    /// [UpdateNotification::handle_update_result] should write notification_file,
    /// if no file has been written before
    #[test]
    fn handle_update_result_writes_file_if_file_missing() {
        let temp_dir = tempdir().unwrap();
        let notification_file = temp_dir.path().join("notification_file");

        UpdateNotification::handle_update_result(
            Ok(UpdateCheckResult::RefreshNotificationFile(
                notification_file.clone(),
            )),
            &Some(InstallerChannel::Stable),
        );

        serde_json::from_str::<LastUpdateCheck>(&fs::read_to_string(notification_file).unwrap())
            .unwrap();
    }

    /// [UpdateNotificationError::WeMayHaveMessedUp] errors should be sent to sentry
    #[test]
    fn test_handle_update_result_sends_error_to_sentry() {
        let events = with_captured_events(|| {
            UpdateNotification::handle_update_result(
                Err(UpdateNotificationError::WeMayHaveMessedUp(anyhow!("error"))),
                &None,
            );
        });
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].exception.values[0].value.as_ref().unwrap(),
            "Failed to check for CLI updates: error"
        );
    }

    /// [UpdateNotificationError::Io] errors should not be sent to sentry
    #[test]
    fn test_handle_update_result_does_not_send_io_error_to_sentry() {
        let events = with_captured_events(|| {
            UpdateNotification::handle_update_result(
                Err(UpdateNotificationError::Io(io::Error::from(
                    io::ErrorKind::UnexpectedEof,
                ))),
                &None,
            );
        });
        assert_eq!(events.len(), 0);
    }

    /// When notification_file contains a recent timestamp,
    /// UpdateNotification::testable_check_for_update should return None
    #[tokio::test]
    async fn test_check_for_update_returns_none_if_already_notified() {
        let temp_dir = tempdir().unwrap();
        let notification_file = temp_dir.path().join(UPDATE_NOTIFICATION_FILE_NAME);
        fs::write(
            &notification_file,
            serde_json::to_string(&LastUpdateCheck {
                last_update_check: OffsetDateTime::now_utc(),
            })
            .unwrap(),
        )
        .unwrap();

        let result = UpdateNotification::check_for_update_inner(
            notification_file,
            &FLOX_VERSION,
            async { panic!() },
            UPDATE_NOTIFICATION_EXPIRY,
        )
        .await;

        assert_eq!(result.unwrap(), UpdateCheckResult::Skipped);
    }

    /// When notification_file contains an old timestamp,
    /// testable_check_for_update should return an UpdateNotification
    #[tokio::test]
    async fn test_check_for_update_returns_some_if_expired() {
        let temp_dir = tempdir().unwrap();
        let notification_file = temp_dir.path().join(UPDATE_NOTIFICATION_FILE_NAME);
        fs::write(
            &notification_file,
            serde_json::to_string(&LastUpdateCheck {
                last_update_check: OffsetDateTime::now_utc()
                    - UPDATE_NOTIFICATION_EXPIRY
                    - Duration::seconds(1),
            })
            .unwrap(),
        )
        .unwrap();

        let latest_version: String = "1000.0.0".to_string();

        let result = UpdateNotification::check_for_update_inner(
            notification_file.clone(),
            &FLOX_VERSION,
            async { Ok(latest_version.clone()) },
            UPDATE_NOTIFICATION_EXPIRY,
        )
        .await;

        assert_eq!(
            result.unwrap(),
            UpdateCheckResult::UpdateAvailable(UpdateNotification {
                notification_file,
                new_version: latest_version.clone(),
            })
        );
    }

    /// When there's no existing notification_file,
    /// testable_check_for_update should return an UpdateNotification
    #[tokio::test]
    async fn test_check_for_update_returns_some_if_no_notification_file() {
        let temp_dir = tempdir().unwrap();
        let notification_file = temp_dir.path().join(UPDATE_NOTIFICATION_FILE_NAME);

        let result = UpdateNotification::check_for_update_inner(
            notification_file.clone(),
            &FLOX_VERSION,
            async { Ok("1000.0.0".to_string()) },
            UPDATE_NOTIFICATION_EXPIRY,
        )
        .await;

        assert_eq!(
            result.unwrap(),
            UpdateCheckResult::UpdateAvailable(UpdateNotification {
                notification_file,
                new_version: "1000.0.0".to_string()
            })
        );
    }

    /// testable_check_for_update fails when get_latest_version_function doesn't
    /// return something that looks like a version
    #[tokio::test]
    async fn test_check_for_update_fails_for_bad_version() {
        let temp_dir = tempdir().unwrap();
        let notification_file = temp_dir.path().join(UPDATE_NOTIFICATION_FILE_NAME);

        let result = UpdateNotification::check_for_update_inner(
            notification_file.clone(),
            &FLOX_VERSION,
            async { Ok("bad".to_string()) },
            UPDATE_NOTIFICATION_EXPIRY,
        )
        .await;

        match result {
            Err(UpdateNotificationError::WeMayHaveMessedUp(e)) => {
                assert!(e.to_string().contains("version 'bad' is invalid"))
            },
            _ => panic!(),
        }
    }

    /// [UpdateNotification::check_for_update_inner] fails when `get_latest_version_function`
    /// doesn't return something that looks like a version
    #[tokio::test]
    async fn test_check_for_update_returns_no_update_for_invalid_version() {
        let temp_dir = tempdir().unwrap();
        let notification_file = temp_dir.path().join(UPDATE_NOTIFICATION_FILE_NAME);

        let result = UpdateNotification::check_for_update_inner(
            notification_file.clone(),
            &FLOX_VERSION,
            async { Ok("not-a-version".into()) },
            UPDATE_NOTIFICATION_EXPIRY,
        )
        .await;

        assert!(
            matches!(result, Err(UpdateNotificationError::WeMayHaveMessedUp(_))),
            "{result:?}"
        );
    }

    /// [UpdateNotification::check_for_update_inner] returns
    /// [UpdateCheckResult::MissingNotificationFile] if no update is available
    ///  but the notification file is missing
    #[tokio::test]
    async fn test_check_for_update_returns_missing_notification_file() {
        let temp_dir = tempdir().unwrap();
        let notification_file = temp_dir.path().join(UPDATE_NOTIFICATION_FILE_NAME);

        let result = UpdateNotification::check_for_update_inner(
            notification_file.clone(),
            // In development FLOX_VERSION may be 0.0.0-dirty which can't be
            // compared so the test fails
            &"0.0.0".parse().unwrap(),
            async { Ok("0.0.0".to_string()) },
            UPDATE_NOTIFICATION_EXPIRY,
        )
        .await;

        assert_eq!(
            result.unwrap(),
            UpdateCheckResult::RefreshNotificationFile(notification_file)
        );
    }

    // test that update_instructions provides default message when update-instructions.txt file
    // does not exits
    #[test]
    fn test_update_instructions_default_message() {
        let message = UpdateNotification::update_instructions("does-not-exists", &None);
        assert!(message == DEFAULT_UPDATE_INSTRUCTIONS);
    }

    // test that update_instructions returns the message from update-instructions.txt file
    #[test]
    fn test_update_instructions_custom_message() {
        let temp_dir = tempdir().unwrap();
        let update_instructions_file = temp_dir.path().join("update-instructions.txt");
        let custom_message = "This are custom update instructions";

        fs::write(&update_instructions_file, custom_message).unwrap();

        let message = UpdateNotification::update_instructions(
            update_instructions_file.to_str().unwrap(),
            &Some(InstallerChannel::Stable),
        );
        assert!(message.contains(custom_message));
    }
}
