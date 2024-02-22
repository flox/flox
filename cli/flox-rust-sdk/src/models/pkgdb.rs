use std::env;
use std::fmt::Display;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use log::debug;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

// This is the `PKGDB` path that we actually use.
// This is set once and prefers the `PKGDB` env variable, but will use
// the fallback to the binary available at build time if it is unset.
pub static PKGDB_BIN: Lazy<String> =
    Lazy::new(|| env::var("PKGDB_BIN").unwrap_or(env!("PKGDB_BIN").to_string()));

/// Error codes emitted by pkgdb
/// matching the definitions in `pkgdb/include/flox/core/exceptions.hh`
/// for brevity, only the ones we expliticly match are included here.
/// TODO: find a way to _share_ these constants between the Rust and C++ code.
pub mod error_codes {
    /// Manifest file has invalid format
    pub const INVALID_MANIFEST_FILE: u64 = 105;
    /// Parsing of the manifest.toml file failed
    pub const TOML_TO_JSON: u64 = 116;
    /// The package is not found in the package database
    pub const RESOLUTION_FAILURE: u64 = 120;
    /// Conflict between two packages
    pub const BUILDENV_CONFLICT: u64 = 122;
    /// The environment is not compatible with the current system
    pub const LOCKFILE_INCOMPATIBLE_SYSTEM: u64 = 123;
    /// The package is not compatible with the current system
    pub const PACKAGE_EVAL_INCOMPATIBLE_SYSTEM: u64 = 124;
    /// The package failed to evaluate
    pub const PACKAGE_EVAL_FAILURE: u64 = 125;
    /// The package failed to build
    pub const PACKAGE_BUILD_FAILURE: u64 = 126;
}

/// The JSON output of a `pkgdb upgrade` call
#[derive(Deserialize)]
pub struct UpgradeResultJSON {
    pub result: UpgradeResultInner,
    pub lockfile: Value,
}

#[derive(Debug, Deserialize)]
pub struct UpgradeResultInner(pub Vec<String>);

/// The JSON output of a `pkgdb buildenv` call
#[derive(Deserialize)]
pub struct BuildEnvResult {
    pub store_path: String,
}

#[derive(Debug)]
pub struct UpgradeResult {
    pub packages: Vec<String>,
    pub store_path: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum CallPkgDbError {
    #[error(transparent)]
    PkgDbError(#[from] PkgDbError),
    #[error("couldn't parse pkgdb error in expected JSON format")]
    ParsePkgDbError(#[source] serde_json::Error),
    #[error("couldn't parse pkgdb output as JSON")]
    ParseJSON(#[source] serde_json::Error),
    #[error("call to pkgdb failed")]
    PkgDbCall(#[source] std::io::Error),
    #[error("couldn't get pkgdb stdout")]
    PkgDbStdout,
    #[error("couldn't get pkgdb stderr")]
    PkgDbStderr,
}

/// Call pkgdb and try to parse JSON or error JSON.
///
/// Error JSON is parsed into a [CallPkgDbError::PkgDbError].
pub fn call_pkgdb(mut pkgdb_cmd: Command) -> Result<Value, CallPkgDbError> {
    let mut proc = pkgdb_cmd
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(CallPkgDbError::PkgDbCall)?;
    let stderr = proc.stderr.take().expect("couldn't get stderr handle");
    let stderr_reader = BufReader::new(stderr);
    stderr_reader
        .lines()
        .map_while(Result::ok)
        .for_each(|line| {
            debug!(target: "pkgdb", "{line}");
        });
    let output = proc.wait_with_output().map_err(CallPkgDbError::PkgDbCall)?;
    // If command fails, try to parse stdout as a PkgDbError
    if !output.status.success() {
        match serde_json::from_slice::<PkgDbError>(&output.stdout) {
            Ok(pkgdb_err) => Err(pkgdb_err)?,
            Err(e) => Err(CallPkgDbError::ParsePkgDbError(e))?,
        }
    // If the command succeeds, try to parse stdout as a JSON value
    } else {
        let json = serde_json::from_slice(&output.stdout).map_err(CallPkgDbError::ParseJSON)?;
        Ok(json)
    }
}

/// A struct representing error messages coming from pkgdb
#[derive(Debug, PartialEq)]
pub struct PkgDbError {
    /// The exit code of pkgdb, can be used to programmatically determine
    /// the category of error.
    pub exit_code: u64,
    /// The generic message for this category of error.
    pub category_message: String,
    /// The more contextual message for the specific error that occurred.
    pub context_message: Option<ContextMsgError>,
}

impl<'de> Deserialize<'de> for PkgDbError {
    // Custom deserializer was likely added to control error messages. If we
    // stop propagating them to the user, we could drop the custom deserializer.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut map = serde_json::Map::<String, Value>::deserialize(deserializer)?;
        let exit_code = map
            .remove("exit_code")
            .ok_or_else(|| serde::de::Error::missing_field("exit_code"))?
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom("exit code is not an unsigned integer"))?;
        let category_message = match map.remove("category_message") {
            Some(m) => m
                .as_str()
                .ok_or_else(|| serde::de::Error::custom("category message was not a string"))
                .map(|m| m.to_owned()),
            None => Err(serde::de::Error::missing_field("category_message")),
        }?;
        let context_message_contents = map
            .remove("context_message")
            .map(|m| {
                m.as_str()
                    .ok_or_else(|| serde::de::Error::custom("context message was not a string"))
                    .map(|m| m.to_owned())
            })
            .transpose()?;
        let caught_message_contents = map
            .remove("caught_message")
            .map(|m| {
                m.as_str()
                    .ok_or_else(|| serde::de::Error::custom("caught message was not a string"))
                    .map(|m| m.to_owned())
            })
            .transpose()?;
        let context_message = context_message_contents.map(|m| ContextMsgError {
            message: m,
            caught: caught_message_contents.map(|m| CaughtMsgError { message: m }),
        });

        debug_assert!(
            map.keys().next().is_none(),
            "unknown field in pkgdb error JSON"
        );

        Ok(PkgDbError {
            exit_code,
            category_message,
            context_message,
        })
    }
}

impl Display for PkgDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.category_message)?;
        Ok(())
    }
}

impl std::error::Error for PkgDbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.context_message
            .as_ref()
            .map(|s| s as &dyn std::error::Error)
    }
}

/// A struct representing the context message from a pkgdb error
#[derive(Debug, PartialEq, Deserialize)]
pub struct ContextMsgError {
    pub message: String,
    pub caught: Option<CaughtMsgError>,
}

impl Display for ContextMsgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ContextMsgError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.caught.as_ref().map(|s| s as &dyn std::error::Error)
    }
}

/// A struct representing the caught message from a pkgdb error
#[derive(Debug, PartialEq, Deserialize)]
pub struct CaughtMsgError {
    pub message: String,
}

impl Display for CaughtMsgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CaughtMsgError {}
