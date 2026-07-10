//! PII-safe classification of a dispatch error into a bounded
//! `(kind, message)` descriptor derived from the error's type, not its
//! rendered string. Both fields are `&'static str`, so user data cannot
//! reach telemetry.

use anyhow::Error;
use flox_rust_sdk::models::environment::EnvironmentError;
use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironmentError;
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironmentError;
use flox_rust_sdk::providers::services::process_compose::ServiceError;

use crate::Exit;
use crate::commands::{EnvironmentSelectError, NoEnvironmentError};

/// A bounded, PII-safe error descriptor. Both fields come from a fixed set
/// of compile-time strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ErrorClass {
    pub kind: &'static str,
    pub message: &'static str,
}

const UNCATEGORIZED: ErrorClass = ErrorClass {
    kind: "uncategorized",
    message: "unclassified error",
};

// Shared so a managed/remote environment error classifies the same whether
// it arrives top-level or wrapped in an `EnvironmentError` variant.
const ENV_MANAGED: ErrorClass = ErrorClass {
    kind: "env_managed",
    message: "managed environment operation failed",
};

const ENV_REMOTE: ErrorClass = ErrorClass {
    kind: "env_remote",
    message: "remote environment operation failed",
};

// Shared so a missing environment classifies the same whether it surfaces as
// an `EnvironmentError` (a `.flox` dir was expected) or an
// `EnvironmentSelectError` (no environment to act on).
const ENV_NOT_FOUND: ErrorClass = ErrorClass {
    kind: "env_not_found",
    message: "environment not found",
};

/// Derive a `(kind, message)` by matching on the error's type/variant.
/// Mirrors the downcast ladder in `main.rs`; keep the two in sync.
pub(crate) fn classify(err: &Error) -> ErrorClass {
    // A deliberate `Err(Exit(..))` is a controlled, already-messaged exit
    // (e.g. an `auth login` rejection or a validation failure), not an unknown
    // error — keep it out of the `uncategorized` bucket.
    if err.downcast_ref::<Exit>().is_some() {
        return ErrorClass {
            kind: "controlled_exit",
            message: "controlled exit",
        };
    }
    if err.downcast_ref::<NoEnvironmentError>().is_some() {
        return ENV_NOT_FOUND;
    }
    if let Some(e) = err.downcast_ref::<EnvironmentError>() {
        return classify_environment(e);
    }
    if err.downcast_ref::<ManagedEnvironmentError>().is_some() {
        return ENV_MANAGED;
    }
    if err.downcast_ref::<RemoteEnvironmentError>().is_some() {
        return ENV_REMOTE;
    }
    if let Some(e) = err.downcast_ref::<EnvironmentSelectError>() {
        return match e {
            // anyhow's downcast matches only the outer stored type
            // (`EnvironmentSelectError`), not the wrapped `EnvironmentError`, so
            // classify the inner here rather than at the top of the ladder.
            EnvironmentSelectError::EnvironmentError(inner) => classify_environment(inner),
            EnvironmentSelectError::EnvNotFound
            | EnvironmentSelectError::EnvNotFoundInCurrentDirectory => ENV_NOT_FOUND,
            _ => ErrorClass {
                kind: "env_select",
                message: "could not select an environment",
            },
        };
    }
    if err.downcast_ref::<ServiceError>().is_some() {
        return ErrorClass {
            kind: "service",
            message: "service operation failed",
        };
    }
    UNCATEGORIZED
}

fn classify_environment(e: &EnvironmentError) -> ErrorClass {
    match e {
        EnvironmentError::ManifestNotFound => ErrorClass {
            kind: "manifest_missing",
            message: "manifest not found",
        },
        EnvironmentError::DotFloxNotFound(_)
        | EnvironmentError::EnvDirNotFound
        | EnvironmentError::EnvPointerNotFound => ENV_NOT_FOUND,
        EnvironmentError::EnvironmentExists(_) => ErrorClass {
            kind: "env_exists",
            message: "environment already exists",
        },
        EnvironmentError::ReadEnvironmentMetadata(_)
        | EnvironmentError::ParseEnvJson(_)
        | EnvironmentError::SerializeEnvJson(_)
        | EnvironmentError::WriteEnvJson(_) => ErrorClass {
            kind: "env_metadata",
            message: "could not read environment metadata",
        },
        EnvironmentError::ManifestError(_) => ErrorClass {
            kind: "manifest_error",
            message: "manifest error",
        },
        EnvironmentError::ManagedEnvironment(_) => ENV_MANAGED,
        EnvironmentError::RemoteEnvironment(_) => ENV_REMOTE,
        _ => ErrorClass {
            kind: "env_other",
            message: "environment operation failed",
        },
    }
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use anyhow::anyhow;
    use flox_rust_sdk::models::environment::EnvironmentError;

    use super::*;
    use crate::Exit;
    use crate::commands::{EnvironmentSelectError, NoEnvironmentError};

    #[test]
    fn controlled_exit_is_not_uncategorized() {
        let err = anyhow::Error::from(Exit(ExitCode::from(1)));
        assert_eq!(classify(&err), ErrorClass {
            kind: "controlled_exit",
            message: "controlled exit",
        });
    }

    #[test]
    fn anyhow_string_is_uncategorized() {
        let err = anyhow!("could not open /Users/alice/secret.toml");
        let class = classify(&err);
        assert_eq!(class, UNCATEGORIZED);
        // A path + username went in; assert neither survives.
        assert!(
            !class.message.contains("alice"),
            "PII leaked: {}",
            class.message
        );
    }

    #[test]
    fn environment_manifest_not_found_classifies() {
        let err = anyhow::Error::from(EnvironmentError::ManifestNotFound);
        assert_eq!(classify(&err), ErrorClass {
            kind: "manifest_missing",
            message: "manifest not found",
        });
    }

    #[test]
    fn dot_flox_not_found_is_env_not_found() {
        let err = anyhow::Error::from(EnvironmentError::DotFloxNotFound(std::path::PathBuf::from(
            "/tmp/project/.flox",
        )));
        assert_eq!(classify(&err), ENV_NOT_FOUND);
    }

    #[test]
    fn env_select_not_found_is_env_not_found() {
        let err = anyhow::Error::from(EnvironmentSelectError::EnvNotFoundInCurrentDirectory);
        assert_eq!(classify(&err), ENV_NOT_FOUND);
    }

    #[test]
    fn env_select_wrapping_dot_flox_not_found_is_env_not_found() {
        let inner =
            EnvironmentError::DotFloxNotFound(std::path::PathBuf::from("/tmp/project/.flox"));
        let err = anyhow::Error::from(EnvironmentSelectError::EnvironmentError(inner));
        assert_eq!(classify(&err), ENV_NOT_FOUND);
    }

    #[test]
    fn no_environment_error_is_env_not_found() {
        let err = anyhow::Error::from(NoEnvironmentError::CurrentDirectory);
        assert_eq!(classify(&err), ENV_NOT_FOUND);
    }

    #[test]
    fn all_outputs_are_pii_safe() {
        let samples = [
            classify(&anyhow!("boom")),
            classify(&anyhow::Error::from(EnvironmentError::ManifestNotFound)),
            classify(&anyhow::Error::from(EnvironmentError::EnvDirNotFound)),
        ];
        for c in samples {
            assert!(
                c.kind
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'_' || b.is_ascii_digit()),
                "bad kind slug: {}",
                c.kind
            );
            assert!(
                !c.message.contains('{') && !c.message.contains('}'),
                "template placeholder leaked into message: {}",
                c.message
            );
            assert!(c.message.len() <= 64, "message too long: {}", c.message);
        }
    }
}
