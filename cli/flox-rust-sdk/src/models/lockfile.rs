use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use flox_types::catalog::System;
use flox_types::version::Version;
use log::debug;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::pkgdb::CallPkgDbError;
use crate::models::environment::CanonicalPath;
use crate::models::pkgdb::{call_pkgdb, BuildEnvResult};

#[derive(Debug, Serialize, Clone, Deserialize, PartialEq)]
pub struct LockedManifest(Value);

impl LockedManifest {
    /// Use pkgdb to lock a manifest
    pub fn lock_manifest(
        pkgdb: &Path,
        manifest_path: &Path,
        existing_lockfile_path: Option<CanonicalPath>,
        global_manifest_path: &Path,
    ) -> Result<Self, LockedManifestError> {
        let canonical_manifest_path = manifest_path
            .canonicalize()
            .map_err(|e| LockedManifestError::BadManifestPath(e, manifest_path.to_path_buf()))?;

        let mut pkgdb_cmd = Command::new(pkgdb);
        pkgdb_cmd
            .args(["manifest", "lock"])
            .arg("--ga-registry")
            .arg("--global-manifest")
            .arg(global_manifest_path)
            .arg("--manifest")
            .arg(canonical_manifest_path);
        if let Some(canonical_lockfile_path) = existing_lockfile_path {
            pkgdb_cmd.arg("--lockfile").arg(canonical_lockfile_path);
        }

        debug!("locking manifest with command: {pkgdb_cmd:?}");
        call_pkgdb(pkgdb_cmd)
            .map_err(LockedManifestError::LockManifest)
            .map(Self)
    }

    /// Build a locked manifest
    ///
    /// if a gcroot_out_link_path is provided,
    /// the environment will be linked to that path and a gcroot will be created
    pub fn build(
        &self,
        pkgdb: &Path,
        gcroot_out_link_path: Option<&Path>,
    ) -> Result<PathBuf, LockedManifestError> {
        let mut pkgdb_cmd = Command::new(pkgdb);
        pkgdb_cmd.arg("buildenv").arg(&self.to_string());

        if let Some(gcroot_out_link_path) = gcroot_out_link_path {
            pkgdb_cmd.args(["--out-link", &gcroot_out_link_path.to_string_lossy()]);
        }

        debug!("building environment with command: {pkgdb_cmd:?}");

        let result: BuildEnvResult =
            serde_json::from_value(call_pkgdb(pkgdb_cmd).map_err(LockedManifestError::BuildEnv)?)
                .map_err(LockedManifestError::ParseBuildEnvOutput)?;

        Ok(PathBuf::from(result.store_path))
    }
}

impl ToString for LockedManifest {
    fn to_string(&self) -> String {
        self.0.to_string()
    }
}

#[derive(Debug, Serialize, Clone, Deserialize, PartialEq)]
pub struct TypedLockedManifest {
    #[serde(rename = "lockfile-version")]
    lockfile_version: Version<0>,

    manifest: Manifest,
    packages: BTreeMap<System, BTreeMap<String, LockedPackage>>,
}

#[derive(Debug, Serialize, Clone, Deserialize, PartialEq)]
struct Manifest {
    install: BTreeMap<String, InstallSpec>,
}

#[derive(Debug, Serialize, Clone, Deserialize, PartialEq)]
struct InstallSpec {
    path: String,
}

#[derive(Debug, Serialize, Clone, Deserialize, PartialEq)]
struct LockedPackage {
    info: PackageInfo,
    priority: usize,
}

#[derive(Debug, Serialize, Clone, Deserialize, PartialEq)]
pub struct PackageInfo {
    pub broken: bool,
    pub license: Option<String>,
    pub pname: String,
    pub unfree: bool,
    pub version: String,
}

impl TryFrom<LockedManifest> for TypedLockedManifest {
    type Error = LockedManifestError;

    fn try_from(value: LockedManifest) -> Result<Self, Self::Error> {
        serde_json::from_value(value.0).map_err(LockedManifestError::ParseLockedManifest)
    }
}

impl TypedLockedManifest {
    /// List all packages in the locked manifest for a given system
    pub fn list_packages(&self, system: &System) -> Vec<InstalledPackage> {
        let mut packages = vec![];
        if let Some(system_packages) = self.packages.get(system) {
            for (name, locked_package) in system_packages {
                packages.push(InstalledPackage {
                    name: name.clone(),
                    // SAFETY: we know that the package is in the manifest because it is locked
                    path: self.manifest.install.get(name).unwrap().path.clone(),
                    info: locked_package.info.clone(),
                    priority: locked_package.priority,
                });
            }
        }
        packages
    }
}

pub struct InstalledPackage {
    pub name: String,
    pub path: String,
    pub info: PackageInfo,
    pub priority: usize,
}

#[derive(Debug, Error)]
pub enum LockedManifestError {
    #[error("failed to lock manifest")]
    LockManifest(#[source] CallPkgDbError),
    #[error("failed to build environment")]
    BuildEnv(#[source] CallPkgDbError),
    #[error("failed to parse buildenv output")]
    ParseBuildEnvOutput(#[source] serde_json::Error),
    #[error("failed to canonicalize manifest path: {0:?}")]
    BadManifestPath(#[source] std::io::Error, PathBuf),
    #[error(transparent)]
    CallPkgDbError(#[from] CallPkgDbError),
    /// when parsing the contents of a locked manifest into a [TypedLockedManifest]
    #[error("failed to parse contents of locked manifest")]
    ParseLockedManifest(#[source] serde_json::Error),
}
