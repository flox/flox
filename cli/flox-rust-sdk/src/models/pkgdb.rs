use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use log::debug;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use super::environment::{CanonicalizeError, UpdateResult};
use crate::flox::Flox;
use crate::models::environment::{
    global_manifest_lockfile_path,
    global_manifest_path,
    CanonicalPath,
};
use crate::models::lockfile::LockedManifest;

// This is the `PKGDB` path that we actually use.
// This is set once and prefers the `PKGDB` env variable, but will use
// the fallback to the binary available at build time if it is unset.
pub static PKGDB_BIN: Lazy<String> =
    Lazy::new(|| env::var("PKGDB_BIN").unwrap_or(env!("PKGDB_BIN").to_string()));

/// The JSON output of a `pkgdb upgrade` call
#[derive(Deserialize)]
pub struct UpgradeResultJSON {
    pub result: UpgradeResult,
    pub lockfile: Value,
}

/// The JSON output of a `pkgdb buildenv` call
#[derive(Deserialize)]
pub struct BuildEnvResult {
    pub store_path: String,
}

#[derive(Deserialize)]
pub struct UpgradeResult(pub Vec<String>);

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
}

/// Call pkgdb and try to parse JSON or error JSON.
///
/// Error JSON is parsed into a [CallPkgDbError::PkgDbError].
pub fn call_pkgdb(mut pkgdb_cmd: Command) -> Result<Value, CallPkgDbError> {
    let output = pkgdb_cmd.output().map_err(CallPkgDbError::PkgDbCall)?;
    // If command fails, try to parse stdout as a PkgDbError
    if !output.status.success() {
        match serde_json::from_slice::<PkgDbError>(&output.stdout) {
            Ok(pkgdb_err) => Err(pkgdb_err)?,
            Err(e) => Err(CallPkgDbError::ParsePkgDbError(e))?,
        }
    // If command succeeds, try to parse stdout as JSON value
    } else {
        let json = serde_json::from_slice(&output.stdout).map_err(CallPkgDbError::ParseJSON)?;
        Ok(json)
    }
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("could not parse lockfile")]
    ParseLockfile(#[source] serde_json::Error),
    #[error("failed to update environment")]
    UpdateFailed(#[source] CallPkgDbError),
    #[error("unexpected output from pkgdb update")]
    ParseUpdateOutput(#[source] serde_json::Error),
    #[error("could not open manifest file")]
    ReadLockfile(#[source] std::io::Error),
    #[error(transparent)]
    BadLockfilePath(CanonicalizeError),
    /// TODO: not sure if the global error's belong in this enum
    #[error("could not serialize global lockfile")]
    SerializeGlobalLockfile(#[source] serde_json::Error),
    #[error("could not write global lockfile")]
    WriteGlobalLockfile(#[source] std::io::Error),
}

/// Wrapper around `pkgdb update`
///
/// lockfile_path does not need to exist
/// TODO: lockfile_path should probably be an Option<CanonicalPath>
pub fn pkgdb_update(
    flox: &Flox,
    manifest_path: Option<impl AsRef<Path>>,
    lockfile_path: impl AsRef<Path>,
    inputs: Vec<String>,
) -> Result<UpdateResult, UpdateError> {
    let lockfile_path = lockfile_path.as_ref();
    let maybe_lockfile = if lockfile_path.exists() {
        debug!("found existing lockfile: {}", lockfile_path.display());
        Some(lockfile_path)
    } else {
        debug!("no existing lockfile found");
        None
    };

    let mut pkgdb_cmd = Command::new(Path::new(&*PKGDB_BIN));
    pkgdb_cmd
        .args(["manifest", "update"])
        .arg("--ga-registry")
        .arg("--global-manifest")
        .arg(global_manifest_path(flox));
    // Optionally add --manifest argument
    if let Some(manifest) = manifest_path {
        pkgdb_cmd.arg("--manifest").arg(manifest.as_ref());
    }
    // Add --lockfile argument if lockfile exists, and parse the old lockfile.
    let old_lockfile = maybe_lockfile
        .map(|lf_path| {
            let canonical_lockfile_path =
                CanonicalPath::new(lf_path).map_err(UpdateError::BadLockfilePath)?;
            pkgdb_cmd.arg("--lockfile").arg(&canonical_lockfile_path);
            serde_json::from_slice(
                &fs::read(canonical_lockfile_path).map_err(UpdateError::ReadLockfile)?,
            )
            .map_err(UpdateError::ParseLockfile)
        })
        .transpose()?;

    pkgdb_cmd.args(inputs);

    debug!("updating lockfile with command: {pkgdb_cmd:?}");
    let lockfile: LockedManifest =
        serde_json::from_value(call_pkgdb(pkgdb_cmd).map_err(UpdateError::UpdateFailed)?)
            .map_err(UpdateError::ParseUpdateOutput)?;

    Ok((old_lockfile, lockfile))
}

/// Update global manifest lockfile and write it.
///
/// TODO: this probably doesn't belong in the pkgdb module but I'm not sure
/// where else to put it.
pub fn update_global_manifest(
    flox: &Flox,
    inputs: Vec<String>,
) -> Result<UpdateResult, UpdateError> {
    let lockfile_path = global_manifest_lockfile_path(flox);
    let (old_lockfile, new_lockfile) = pkgdb_update(flox, None::<PathBuf>, &lockfile_path, inputs)?;

    debug!("writing lockfile to {}", lockfile_path.display());
    std::fs::write(
        lockfile_path,
        serde_json::to_string_pretty(&new_lockfile)
            .map_err(UpdateError::SerializeGlobalLockfile)?,
    )
    .map_err(UpdateError::WriteGlobalLockfile)?;
    Ok((old_lockfile, new_lockfile))
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
