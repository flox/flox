use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type FlakeRef = Value;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use log::debug;
use thiserror::Error;

use super::container_builder::ContainerBuilder;
use super::environment::UpdateResult;
use super::pkgdb::CallPkgDbError;
use crate::data::{CanonicalPath, CanonicalizeError, System, Version};
use crate::flox::Flox;
use crate::models::environment::{global_manifest_lockfile_path, global_manifest_path};
use crate::models::pkgdb::{call_pkgdb, BuildEnvResult, PKGDB_BIN};
use crate::utils::CommandExt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Input {
    pub from: FlakeRef,
    #[serde(flatten)]
    _json: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    pub inputs: BTreeMap<String, Input>,
    #[serde(flatten)]
    _json: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LockedManifest(Value);

impl LockedManifest {
    /// Use pkgdb to lock a manifest
    ///
    /// `existing_lockfile_path` can be either the global lock or an environment's
    /// lockfile
    pub fn lock_manifest(
        pkgdb: &Path,
        manifest_path: &Path,
        existing_lockfile_path: &CanonicalPath,
        global_manifest_path: &Path,
    ) -> Result<Self, LockedManifestError> {
        let canonical_manifest_path =
            CanonicalPath::new(manifest_path).map_err(LockedManifestError::BadManifestPath)?;

        let mut pkgdb_cmd = Command::new(pkgdb);
        pkgdb_cmd
            .args(["manifest", "lock"])
            .arg("--ga-registry")
            .arg("--global-manifest")
            .arg(global_manifest_path)
            .arg("--manifest")
            .arg(canonical_manifest_path)
            .arg("--lockfile")
            .arg(existing_lockfile_path);

        debug!("locking manifest with command: {}", pkgdb_cmd.display());
        call_pkgdb(pkgdb_cmd)
            .map_err(LockedManifestError::LockManifest)
            .map(Self)
    }

    /// Build a locked manifest
    ///
    /// If a gcroot_out_link_path is provided,
    /// the environment will be linked to that path and a gcroot will be created
    pub fn build(
        &self,
        pkgdb: &Path,
        gcroot_out_link_path: Option<&Path>,
        store_path: &Option<PathBuf>,
    ) -> Result<PathBuf, LockedManifestError> {
        let mut pkgdb_cmd = Command::new(pkgdb);
        pkgdb_cmd.arg("buildenv").arg(&self.to_string());

        if let Some(gcroot_out_link_path) = gcroot_out_link_path {
            pkgdb_cmd.args(["--out-link", &gcroot_out_link_path.to_string_lossy()]);
            if let Some(store_path) = store_path {
                pkgdb_cmd.args(["--store-path", &store_path.to_string_lossy()]);
            }
        }

        debug!("building environment with command: {}", pkgdb_cmd.display());

        let result: BuildEnvResult =
            serde_json::from_value(call_pkgdb(pkgdb_cmd).map_err(LockedManifestError::BuildEnv)?)
                .map_err(LockedManifestError::ParseBuildEnvOutput)?;

        Ok(PathBuf::from(result.store_path))
    }

    /// Build a container image from a locked manifest
    /// and write it to a provided sink.
    ///
    /// The sink can be e.g. a [File](std::fs::File), [Stdout](std::io::Stdout),
    /// or an internal buffer.
    pub fn build_container(&self, pkgdb: &Path) -> Result<ContainerBuilder, LockedManifestError> {
        let mut pkgdb_cmd = Command::new(pkgdb);
        pkgdb_cmd
            .arg("buildenv")
            .arg("--container")
            .arg(&self.to_string());

        debug!(
            "building container builder with command: {}",
            pkgdb_cmd.display()
        );
        let result: BuildEnvResult =
            serde_json::from_value(call_pkgdb(pkgdb_cmd).map_err(LockedManifestError::BuildEnv)?)
                .map_err(LockedManifestError::ParseBuildEnvOutput)?;

        let container_builder_path = PathBuf::from(result.store_path);

        Ok(ContainerBuilder::new(container_builder_path))
    }

    /// Wrapper around `pkgdb update`
    ///
    /// lockfile_path does not need to exist
    /// TODO: lockfile_path should probably be an Option<CanonicalPath>
    pub fn update_manifest(
        flox: &Flox,
        manifest_path: Option<impl AsRef<Path>>,
        lockfile_path: impl AsRef<Path>,
        inputs: Vec<String>,
    ) -> Result<UpdateResult, LockedManifestError> {
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
                    CanonicalPath::new(lf_path).map_err(LockedManifestError::BadLockfilePath)?;
                pkgdb_cmd.arg("--lockfile").arg(&canonical_lockfile_path);
                Self::read_from_file(&canonical_lockfile_path)
            })
            .transpose()?;

        pkgdb_cmd.args(inputs);

        debug!("updating lockfile with command: {}", pkgdb_cmd.display());
        let lockfile: LockedManifest =
            LockedManifest(call_pkgdb(pkgdb_cmd).map_err(LockedManifestError::UpdateFailed)?);

        Ok(UpdateResult {
            new_lockfile: lockfile,
            old_lockfile,
            store_path: None,
        })
    }

    /// Update global manifest lockfile and write it.
    pub fn update_global_manifest(
        flox: &Flox,
        inputs: Vec<String>,
    ) -> Result<UpdateResult, LockedManifestError> {
        let lockfile_path = global_manifest_lockfile_path(flox);
        let UpdateResult {
            new_lockfile,
            old_lockfile,
            store_path,
        } = Self::update_manifest(flox, None::<PathBuf>, &lockfile_path, inputs)?;

        debug!("writing lockfile to {}", lockfile_path.display());
        std::fs::write(
            lockfile_path,
            serde_json::to_string_pretty(&new_lockfile)
                .map_err(LockedManifestError::SerializeGlobalLockfile)?,
        )
        .map_err(LockedManifestError::WriteGlobalLockfile)?;
        Ok(UpdateResult {
            new_lockfile,
            old_lockfile,
            store_path,
        })
    }

    /// Creates the global lockfile if it doesn't exist and returns its path.
    pub fn ensure_global_lockfile(flox: &Flox) -> Result<PathBuf, LockedManifestError> {
        let global_lockfile_path = global_manifest_lockfile_path(flox);
        if !global_lockfile_path.exists() {
            debug!("Global lockfile does not exist, updating to create one");
            Self::update_global_manifest(flox, vec![])?;
        }
        Ok(global_lockfile_path)
    }

    pub fn read_from_file(path: &CanonicalPath) -> Result<Self, LockedManifestError> {
        let contents = fs::read(path).map_err(LockedManifestError::ReadLockfile)?;
        serde_json::from_slice(&contents).map_err(LockedManifestError::ParseLockfile)
    }

    pub fn check_lockfile(
        path: &CanonicalPath,
    ) -> Result<Vec<LockfileCheckWarning>, LockedManifestError> {
        let mut pkgdb_cmd = Command::new(Path::new(&*PKGDB_BIN));
        pkgdb_cmd
            .args(["manifest", "check"])
            .arg("--lockfile")
            .arg(path.as_os_str());

        debug!("checking lockfile with command: {}", pkgdb_cmd.display());

        let value = call_pkgdb(pkgdb_cmd).map_err(LockedManifestError::CheckLockfile)?;
        let warnings: Vec<LockfileCheckWarning> =
            serde_json::from_value(value).map_err(LockedManifestError::ParseCheckWarnings)?;

        Ok(warnings)
    }
}

impl ToString for LockedManifest {
    fn to_string(&self) -> String {
        self.0.to_string()
    }
}

/// An environment (or global) lockfile.
///
/// This struct is meant **for reading only**.
///
/// It serves as a typed representation of the lockfile json produced by pkgdb.
/// Parsing of the lockfile is done in [TypedLockedManifest::try_from]
/// and should be as late as possible.
/// Where possible, use the opaque [LockedManifest] instead of this struct
/// to avoid incompatibility issues with the authoritative definition in C++.
///
/// In the optimal case the lockfile schema can be inferred from a common
/// or `pkgdb`-defined schema.
///
/// This struct is used as the format to communicate with pkgdb.
/// Many pkgdb commands will need to pass some of the information in the
/// lockfile through to Rust.
///
/// And some commands (i.e. `list`) will need to read lockfiles
/// to get information about the environment without having to call `pkgdb`.
///
/// Although we could selectively pass fields through,
/// I'm hoping it will be easier to parse the entirety of the lockfile in Rust,
/// rather than defining a separate set of fields for each different pkgdb
/// command.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TypedLockedManifest {
    #[serde(rename = "lockfile-version")]
    lockfile_version: Version<0>,
    packages: BTreeMap<System, BTreeMap<String, Option<LockedPackage>>>,
    registry: Registry,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct LockedPackage {
    info: PackageInfo,
    #[serde(rename = "attr-path")]
    abs_path: Vec<String>,
    priority: usize,
}

impl LockedPackage {
    pub fn rel_path(&self) -> String {
        self.abs_path
            .iter()
            .skip(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(".")
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PackageInfo {
    pub description: Option<String>,
    pub broken: bool,
    pub license: Option<String>,
    pub pname: String,
    pub unfree: bool,
    pub version: Option<String>,
}

impl TryFrom<LockedManifest> for TypedLockedManifest {
    type Error = LockedManifestError;

    fn try_from(value: LockedManifest) -> Result<Self, Self::Error> {
        serde_json::from_value(value.0).map_err(LockedManifestError::ParseLockedManifest)
    }
}

impl TypedLockedManifest {
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// List all packages in the locked manifest for a given system
    pub fn list_packages(&self, system: &System) -> Vec<InstalledPackage> {
        let mut packages = vec![];
        if let Some(system_packages) = self.packages.get(system) {
            for (name, locked_package) in system_packages {
                if let Some(locked_package) = locked_package {
                    packages.push(InstalledPackage {
                        name: name.clone(),
                        rel_path: locked_package.rel_path(),
                        info: locked_package.info.clone(),
                        priority: locked_package.priority,
                    });
                };
            }
        }
        packages
    }
}

pub struct InstalledPackage {
    pub name: String,
    pub rel_path: String,
    pub info: PackageInfo,
    pub priority: usize,
}

#[derive(Debug, Error)]
pub enum LockedManifestError {
    #[error("failed to lock manifest")]
    LockManifest(#[source] CallPkgDbError),
    #[error("failed to check lockfile")]
    CheckLockfile(#[source] CallPkgDbError),
    #[error("failed to build environment")]
    BuildEnv(#[source] CallPkgDbError),
    #[error("failed to parse check warnings")]
    ParseCheckWarnings(#[source] serde_json::Error),
    #[error("package is unsupported for this sytem")]
    UnsupportedPackageWithDocLink(#[source] CallPkgDbError),
    #[error("failed to build container builder")]
    CallContainerBuilder(#[source] std::io::Error),
    #[error("failed to write container builder to sink")]
    WriteContainer(#[source] std::io::Error),
    #[error("failed to parse buildenv output")]
    ParseBuildEnvOutput(#[source] serde_json::Error),
    #[error("failed to update environment")]
    UpdateFailed(#[source] CallPkgDbError),
    #[error(transparent)]
    BadManifestPath(CanonicalizeError),
    #[error(transparent)]
    BadLockfilePath(CanonicalizeError),
    #[error("could not open manifest file")]
    ReadLockfile(#[source] std::io::Error),
    /// when parsing the contents of a lockfile into a [LockedManifest]
    #[error("could not parse lockfile")]
    ParseLockfile(#[source] serde_json::Error),
    /// when parsing a [LockedManifest] into a [TypedLockedManifest]
    #[error("failed to parse contents of locked manifest")]
    ParseLockedManifest(#[source] serde_json::Error),
    #[error("could not serialize global lockfile")]
    SerializeGlobalLockfile(#[source] serde_json::Error),
    #[error("could not write global lockfile")]
    WriteGlobalLockfile(#[source] std::io::Error),
}

/// A warning produced by `pkgdb manifest check`
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LockfileCheckWarning {
    pub package: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// Validate that the parser for the locked manifest can handle null values
    /// for the `version`, `license`, and `description` fields.
    #[test]
    fn locked_package_tolerates_null_values() {
        let locked_packages =
            serde_json::from_value::<HashMap<String, LockedPackage>>(serde_json::json!({
                    "complete": {
                        "info": {
                            "description": "A package",
                            "broken": false,
                            "license": "MIT",
                            "pname": "package1",
                            "unfree": false,
                            "version": "1.0.0"
                        },
                        "attr-path": ["package1"],
                        "priority": 0
                    },
                    "missing_version": {
                        "info": {
                            "description": "Another package",
                            "broken": false,
                            "license": "MIT",
                            "pname": "package2",
                            "unfree": false,
                            "version": null
                        },
                        "attr-path": ["package2"],
                        "priority": 0
                    },
                    "missing_license": {
                        "info": {
                            "description": "Another package",
                            "broken": false,
                            "license": null,
                            "pname": "package3",
                            "unfree": false,
                            "version": "1.0.0"
                        },
                        "attr-path": ["package3"],
                        "priority": 0
                    },
                    "missing_description": {
                        "info": {
                            "description": null,
                            "broken": false,
                            "license": "MIT",
                            "pname": "package4",
                            "unfree": false,
                            "version": "1.0.0"
                        },
                        "attr-path": ["package4"],
                        "priority": 0
                    },
            }))
            .unwrap();

        assert_eq!(
            locked_packages["complete"].info.version.as_deref(),
            Some("1.0.0")
        );
        assert_eq!(
            locked_packages["complete"].info.license.as_deref(),
            Some("MIT")
        );
        assert_eq!(
            locked_packages["complete"].info.description.as_deref(),
            Some("A package")
        );

        assert_eq!(
            locked_packages["missing_version"].info.version.as_deref(),
            None
        );
        assert_eq!(
            locked_packages["missing_license"].info.license.as_deref(),
            None
        );
        assert_eq!(
            locked_packages["missing_description"]
                .info
                .description
                .as_deref(),
            None
        );
    }
}
