use std::env;
use std::fmt::Display;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use log::debug;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use super::lockfile::FlakeRef;

// This is the `PKGDB` path that we actually use.
// This is set once and prefers the `PKGDB` env variable, but will use
// the fallback to the binary available at build time if it is unset.
pub static PKGDB_BIN: Lazy<String> =
    Lazy::new(|| env::var("PKGDB_BIN").unwrap_or(env!("PKGDB_BIN").to_string()));
pub static NIX_PKG_BIN: Lazy<String> =
    Lazy::new(|| env::var("NIX_PKG").unwrap_or(env!("NIX_PKG").to_string()) + "/bin");
pub static GIT_PKG_BIN: Lazy<String> =
    Lazy::new(|| env::var("GIT_PKG").unwrap_or(env!("GIT_PKG").to_string()) + "/bin");

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
    /// The package does not pass the options check
    pub const BAD_PACKAGE_FAILURE: u64 = 127;
    /// Failed to build the activation script, possibly due to an I/O failure
    pub const ACTIVATION_SCRIPT_BUILD_FAILURE: u64 = 128;
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
    #[error("internal error: {0}")]
    SomethingElse(String),
}

/// Call pkgdb and try to parse JSON or error JSON.
///
/// Error JSON is parsed into a [CallPkgDbError::PkgDbError].
pub fn call_pkgdb(mut pkgdb_cmd: Command) -> Result<Value, CallPkgDbError> {
    // Configure pkgdb PATH with exact versions of everything it needs.
    //
    // Nix itself isn't pure, which is to say that it isn't built with a
    // reference to `git` in its closure, so correspondingly depends upon
    // finding it in $PATH to function. Funnily enough it is also not built
    // to know where `nix` itself resides, and again relies on $PATH for that.
    //
    // This just isn't OK for us as we're looking for flox to operate reliably
    // in "hostile" environments, which includes situation where a user may
    // redefine or blat their $PATH variable entirely, so we always invoke
    // pkgdb with an explicit PATH of our making.
    let pkgdb_paths = [
        Path::new(&*NIX_PKG_BIN),
        Path::new(&*GIT_PKG_BIN),
        // It really shouldn't be necessary to append $PATH,
        // ... so we won't.
    ];
    let pkgdb_path =
        env::join_paths(pkgdb_paths.iter()).expect("nix or git bin dir contain invalid characters");
    let mut proc = pkgdb_cmd
        .env("PATH", pkgdb_path)
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(CallPkgDbError::PkgDbCall)?;
    let stderr = proc.stderr.take().expect("couldn't get stderr handle");
    let stderr_reader = BufReader::new(stderr);
    let stdout = proc.stdout.take().expect("couldn't get stdout handle");
    let mut stdout_reader = BufReader::new(stdout);

    let pkgdb_output = std::thread::scope(|s| {
        let stderr_thread = s.spawn(move || {
            stderr_reader
                .lines()
                .map_while(Result::ok)
                .for_each(|line| {
                    debug!(target: "pkgdb", "{line}");
                });
        });
        let stdout_thread = s.spawn(move || {
            let mut contents = String::new();
            tracing::debug!("reading pkgdb stdout");
            let bytes_read = stdout_reader.read_to_string(&mut contents);
            bytes_read.map(|_| contents)
        });
        tracing::trace!("waiting for background threads to finish");
        let _ = stderr_thread.join();
        let stdout_res = stdout_thread.join();
        tracing::trace!("done waiting for background threads");
        stdout_res
    });
    let Ok(stdout_contents) = pkgdb_output else {
        // Something went wrong in one of the background threads
        return Err(CallPkgDbError::SomethingElse(
            "failed to process pkgdb output".into(),
        ));
    };
    tracing::trace!("waiting for the pkgdb process to exit");
    let _wait_res = proc.wait();
    match stdout_contents {
        Ok(json) => match serde_json::from_str::<PkgDbError>(&json) {
            Ok(pkgdb_err) => Err(CallPkgDbError::PkgDbError(pkgdb_err)),
            Err(_) => serde_json::from_str(&json).map_err(CallPkgDbError::ParseJSON),
        },
        Err(e) => Err(CallPkgDbError::PkgDbCall(e)),
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

#[derive(Debug, Error)]
pub enum ScrapeError {
    #[error(transparent)]
    CallPkgDb(#[from] CallPkgDbError),
    #[error("couldn't serialize FlakeRef")]
    ParseJSON(#[source] serde_json::Error),
}
pub fn scrape_input(input: &FlakeRef) -> Result<(), ScrapeError> {
    let mut pkgdb_cmd = Command::new(Path::new(&*PKGDB_BIN));
    // TODO: this works for nixpkgs, but it won't work for anything else that is not exposing "legacyPackages"
    pkgdb_cmd
        .args(["scrape"])
        .arg(serde_json::to_string(&input).map_err(ScrapeError::ParseJSON)?)
        .arg("legacyPackages");

    debug!("scraping input: {pkgdb_cmd:?}");
    call_pkgdb(pkgdb_cmd)?;
    Ok(())
}
