use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use log::debug;
use pollster::FutureExt;
use thiserror::Error;

use super::{
    copy_dir_recursive,
    CanonicalizeError,
    InstallationAttempt,
    MigrationInfo,
    UninstallationAttempt,
    UpdateResult,
    UpgradeError,
    LOCKFILE_FILENAME,
    MANIFEST_FILENAME,
};
use crate::data::CanonicalPath;
use crate::flox::Flox;
use crate::models::container_builder::ContainerBuilder;
use crate::models::environment::{call_pkgdb, global_manifest_path};
use crate::models::lockfile::{
    LockedManifest,
    LockedManifestCatalog,
    LockedManifestError,
    LockedManifestPkgdb,
    LockedPackageCatalog,
    ResolutionFailure,
};
use crate::models::manifest::{
    insert_packages,
    remove_packages,
    ManifestError,
    PackageToInstall,
    RawManifest,
    TomlEditError,
    TypedManifest,
    TypedManifestCatalog,
    MANIFEST_VERSION_KEY,
};
use crate::models::pkgdb::{
    error_codes,
    CallPkgDbError,
    PkgDbError,
    UpgradeResult,
    UpgradeResultJSON,
    PKGDB_BIN,
};
use crate::providers::catalog::{self, ClientTrait};
use crate::utils::CommandExt;

pub struct ReadOnly {}
struct ReadWrite {}

/// A view of an environment directory
/// that can be used to build, link, and edit the environment.
///
/// This is a generic file based implementation that should be
/// used by implementations of [super::Environment].
pub struct CoreEnvironment<State = ReadOnly> {
    /// A generic environment directory containing
    /// `manifest.toml` and optionally `manifest.lock`,
    /// as well as any assets consumed by the environment.
    ///
    /// Commonly /.../.flox/env/
    env_dir: PathBuf,
    _state: State,
}

impl<State> CoreEnvironment<State> {
    /// Get the underlying path to the environment directory
    pub fn path(&self) -> &Path {
        &self.env_dir
    }

    /// Get the manifest file
    pub fn manifest_path(&self) -> PathBuf {
        self.env_dir.join(MANIFEST_FILENAME)
    }

    /// Get the path to the lockfile
    ///
    /// Note: may not exist
    pub fn lockfile_path(&self) -> PathBuf {
        self.env_dir.join(LOCKFILE_FILENAME)
    }

    /// Read the manifest file
    pub fn manifest_content(&self) -> Result<String, CoreEnvironmentError> {
        fs::read_to_string(self.manifest_path()).map_err(CoreEnvironmentError::OpenManifest)
    }

    pub fn manifest(&self) -> Result<TypedManifest, CoreEnvironmentError> {
        toml::from_str(&self.manifest_content()?).map_err(CoreEnvironmentError::DeserializeManifest)
    }

    /// Lock the environment.
    ///
    /// When a catalog client is provided, the catalog will be used to lock any
    /// "V1" manifest.
    /// Without a catalog client, only "V0" manifests can be locked using the pkgdb.
    /// If a "V1" manifest is locked without a catalog client, an error will be returned.
    ///
    /// This re-writes the lock if it exists.
    ///
    /// Technically this does write to disk as a side effect for now.
    /// It's included in the [ReadOnly] struct for ergonomic reasons
    /// and because it doesn't modify the manifest.
    ///
    /// todo: should we always write the lockfile to disk?
    pub fn lock(&mut self, flox: &Flox) -> Result<LockedManifest, CoreEnvironmentError> {
        let manifest = self.manifest()?;

        let lockfile = match manifest {
            TypedManifest::Pkgdb(_) => {
                tracing::debug!("using pkgdb to lock");
                LockedManifest::Pkgdb(self.lock_with_pkgdb(flox)?)
            },
            TypedManifest::Catalog(manifest) => {
                let Some(ref client) = flox.catalog_client else {
                    return Err(CoreEnvironmentError::CatalogClientMissing);
                };
                tracing::debug!("using catalog client to lock");
                LockedManifest::Catalog(self.lock_with_catalog_client(client, *manifest)?)
            },
        };

        let environment_lockfile_path = self.lockfile_path();

        // Write the lockfile to disk
        // todo: do we always want to do this?
        debug!(
            "generated lockfile, writing to {}",
            environment_lockfile_path.display()
        );
        std::fs::write(
            &environment_lockfile_path,
            serde_json::to_string_pretty(&lockfile).unwrap(),
        )
        .map_err(CoreEnvironmentError::WriteLockfile)?;

        Ok(lockfile)
    }

    /// Lock the environment with the pkgdb
    ///
    /// Passes the manifest and the existing lockfile to `pkgdb manifest lock`.
    /// The lockfile is used to lock the underlying package registry.
    /// If the environment has no lockfile, the global lockfile is used as a base instead.
    fn lock_with_pkgdb(
        &mut self,
        flox: &Flox,
    ) -> Result<LockedManifestPkgdb, CoreEnvironmentError> {
        let manifest_path = self.manifest_path();
        let environment_lockfile_path = self.lockfile_path();
        let existing_lockfile_path = if environment_lockfile_path.exists() {
            debug!(
                "found existing lockfile: {}",
                environment_lockfile_path.display()
            );
            environment_lockfile_path.clone()
        } else {
            debug!("no existing lockfile found, using the global lockfile as a base");
            // Use the global lock so we're less likely to kick off a pkgdb
            // scrape in e.g. an install.
            LockedManifestPkgdb::ensure_global_lockfile(flox)
                .map_err(CoreEnvironmentError::LockedManifest)?
        };
        let lockfile_path = CanonicalPath::new(existing_lockfile_path)
            .map_err(CoreEnvironmentError::BadLockfilePath)?;

        let lockfile = LockedManifestPkgdb::lock_manifest(
            Path::new(&*PKGDB_BIN),
            &manifest_path,
            &lockfile_path,
            &global_manifest_path(flox),
        )
        .map_err(CoreEnvironmentError::LockedManifest)?;
        Ok(lockfile)
    }

    /// Lock the environment with the catalog client
    ///
    /// If a lockfile exists, it is used as a base.
    /// If the manifest should be locked without a base,
    /// remove the lockfile before calling this function or use [Self::upgrade].
    fn lock_with_catalog_client(
        &self,
        client: &catalog::Client,
        manifest: TypedManifestCatalog,
    ) -> Result<LockedManifestCatalog, CoreEnvironmentError> {
        let existing_lockfile = 'lockfile: {
            let Ok(lockfile_path) = CanonicalPath::new(self.lockfile_path()) else {
                break 'lockfile None;
            };
            let lockfile = LockedManifest::read_from_file(&lockfile_path)
                .map_err(CoreEnvironmentError::LockedManifest)?;
            match lockfile {
                LockedManifest::Catalog(lockfile) => Some(lockfile),
                _ => {
                    // This will be the case when performing a migration
                    debug!(
                        "Found version 1 manifest, but lockfile doesn't match: Ignoring lockfile."
                    );
                    None
                },
            }
        };

        LockedManifestCatalog::lock_manifest(&manifest, existing_lockfile.as_ref(), client)
            .block_on()
            .map_err(CoreEnvironmentError::LockedManifest)
    }

    /// Build the environment.
    ///
    /// Technically this does write to disk as a side effect for now.
    /// It's included in the [ReadOnly] struct for ergonomic reasons
    /// and because it doesn't modify the manifest.
    ///
    /// Does not lock the manifest or link the environment to an out path.
    /// Each should be done explicitly if necessary by the caller
    /// using [Self::lock] and [Self::link]:
    ///
    /// ```no_run
    /// # use flox_rust_sdk::models::environment::CoreEnvironment;
    /// # use flox_rust_sdk::flox::Flox;
    /// let flox: Flox = unimplemented!();
    /// let core_env: CoreEnvironment = unimplemented!();
    ///
    /// core_env.lock(&flox).unwrap();
    /// let store_path = core_env.build(&flox).unwrap();
    /// core_env
    ///     .link(&flox, "/path/to/out-link", &Some(store_path))
    ///     .unwrap();
    /// ```
    #[must_use = "don't discard the store path of built environments"]
    pub fn build(&mut self, flox: &Flox) -> Result<PathBuf, CoreEnvironmentError> {
        let lockfile_path = CanonicalPath::new(self.lockfile_path())
            .map_err(CoreEnvironmentError::BadLockfilePath)?;
        let lockfile = LockedManifest::read_from_file(&lockfile_path)
            .map_err(CoreEnvironmentError::LockedManifest)?;

        debug!(
            "building environment: system={}, lockfilePath={}",
            &flox.system,
            lockfile_path.display()
        );

        let store_path = lockfile
            .build(Path::new(&*PKGDB_BIN), None, &None)
            .map_err(CoreEnvironmentError::LockedManifest)?;

        debug!(
            "built locked environment, store path={}",
            store_path.display()
        );

        Ok(store_path)
    }

    /// Creates a [ContainerBuilder] from the environment.
    ///
    /// The sink is typically a [File](std::fs::File), [Stdout](std::io::Stdout)
    /// but can be any type that implements [Write](std::io::Write).
    ///
    /// While container _images_ can be created on any platform,
    /// only linux _containers_ can be run with `docker` or `podman`.
    /// Building an environment for linux on a non-linux platform (macos),
    /// will likely fail unless all packages in the environment can be substituted.
    ///
    /// There are mitigations for this, such as building within a VM or container.
    /// Such solutions are out of scope at this point.
    /// Until then, this function will error with [CoreEnvironmentError::ContainerizeUnsupportedSystem]
    /// if the environment is not linux.
    ///
    /// [Self::lock]s if necessary.
    ///
    /// Technically this does write to disk as a side effect (i.e. by locking).
    /// It's included in the [ReadOnly] struct for ergonomic reasons
    /// and because it doesn't modify the manifest.
    ///
    /// todo: should we always write the lockfile to disk?
    pub fn build_container(
        &mut self,
        flox: &Flox,
    ) -> Result<ContainerBuilder, CoreEnvironmentError> {
        if std::env::consts::OS != "linux" {
            return Err(CoreEnvironmentError::ContainerizeUnsupportedSystem(
                std::env::consts::OS.to_string(),
            ));
        }

        let lockfile = self.lock(flox)?;

        debug!(
            "building container: system={}, lockfilePath={}",
            &flox.system,
            self.lockfile_path().display()
        );

        let builder = lockfile
            .build_container(Path::new(&*PKGDB_BIN))
            .map_err(CoreEnvironmentError::LockedManifest)?;
        Ok(builder)
    }

    /// Create a new out-link for the environment at the given path.
    /// Optionally a store path to the built environment can be provided,
    /// to avoid building the environment again.
    /// Such a store path can be obtained e.g. from [Self::build].
    ///
    /// Builds the environment if necessary.
    ///
    /// Like [Self::build], this requires the environment to be locked.
    /// This method will _not_ create or update the lockfile.
    ///
    /// Errors if the environment  is not locked or cannot be built.
    ///
    /// TODO: should we always build implicitly?
    pub fn link(
        &mut self,
        flox: &Flox,
        out_link_path: impl AsRef<Path>,
        store_path: &Option<PathBuf>,
    ) -> Result<(), CoreEnvironmentError> {
        let lockfile_path = CanonicalPath::new(self.lockfile_path())
            .map_err(CoreEnvironmentError::BadLockfilePath)?;
        let lockfile = LockedManifest::read_from_file(&lockfile_path)
            .map_err(CoreEnvironmentError::LockedManifest)?;

        debug!(
            "linking environment: system={}, lockfilePath={}, outLinkPath={}",
            &flox.system,
            lockfile_path.display(),
            out_link_path.as_ref().display()
        );

        // Note: when `store_path` is `Some`, `--store-path` is passed to `pkgdb buildenv`
        // which skips the build and only attempts to link the environment.
        lockfile
            .build(
                Path::new(&*PKGDB_BIN),
                Some(out_link_path.as_ref()),
                store_path,
            )
            .map_err(CoreEnvironmentError::LockedManifest)?;
        Ok(())
    }
}

/// Environment modifying methods do not link the new environment to an out path.
/// Linking should be done by the caller.
/// Since files referenced by the environment are ingested into the nix store,
/// the same [CoreEnvironment] instance can be used
/// even if the concrete [super::Environment] tracks the files in a different way
/// such as a git repository or a database.
impl CoreEnvironment<ReadOnly> {
    /// Create a new environment view for the given directory
    ///
    /// This assumes that the directory contains a valid manifest.
    pub fn new(env_dir: impl AsRef<Path>) -> Self {
        CoreEnvironment {
            env_dir: env_dir.as_ref().to_path_buf(),
            _state: ReadOnly {},
        }
    }

    /// Install packages to the environment atomically
    ///
    /// Returns the new manifest content if the environment was modified. Also
    /// returns a map of the packages that were already installed. The installation
    /// will proceed if at least one of the requested packages were added to the
    /// manifest.
    pub fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, CoreEnvironmentError> {
        let current_manifest_contents = self.manifest_content()?;
        let mut installation = insert_packages(&current_manifest_contents, packages)
            .map(|insertion| InstallationAttempt {
                new_manifest: insertion.new_toml.map(|toml| toml.to_string()),
                already_installed: insertion.already_installed,
                store_path: None,
            })
            .map_err(CoreEnvironmentError::ModifyToml)?;
        if let Some(ref new_manifest) = installation.new_manifest {
            let store_path = self.transact_with_manifest_contents(new_manifest, flox)?;
            installation.store_path = Some(store_path);
        }
        Ok(installation)
    }

    /// Uninstall packages from the environment atomically
    ///
    /// Returns true if the environment was modified and false otherwise.
    /// TODO: this should return a list of packages that were actually
    /// uninstalled rather than a bool.
    pub fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<UninstallationAttempt, CoreEnvironmentError> {
        let current_manifest_contents = self.manifest_content()?;
        let toml = remove_packages(&current_manifest_contents, &packages)
            .map_err(CoreEnvironmentError::ModifyToml)?;
        let store_path = self.transact_with_manifest_contents(toml.to_string(), flox)?;
        Ok(UninstallationAttempt {
            new_manifest: Some(toml.to_string()),
            store_path: Some(store_path),
        })
    }

    /// Atomically edit this environment, ensuring that it still builds
    pub fn edit(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<EditResult, CoreEnvironmentError> {
        let old_contents = self.manifest_content()?;

        // skip the edit if the contents are unchanged
        // note: consumers of this function may call [Self::link] separately,
        //       causing an evaluation/build of the environment.
        if contents == old_contents {
            return Ok(EditResult::Unchanged);
        }

        let store_path = self.transact_with_manifest_contents(&contents, flox)?;

        EditResult::new(&old_contents, &contents, Some(store_path))
    }

    /// Atomically edit this environment, without checking that it still builds
    ///
    /// This is unsafe as it can create broken environments!
    /// Used by the implementation of <https://github.com/flox/flox/issues/823>
    /// and may be removed in the future in favor of something like <https://github.com/flox/flox/pull/681>
    pub(crate) fn edit_unsafe(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<Result<EditResult, CoreEnvironmentError>, CoreEnvironmentError> {
        let old_contents = self.manifest_content()?;

        // skip the edit if the contents are unchanged
        // note: consumers of this function may call [Self::link] separately,
        //       causing an evaluation/build of the environment.
        if contents == old_contents {
            return Ok(Ok(EditResult::Unchanged));
        }

        let tempdir = tempfile::tempdir_in(&flox.temp_dir)
            .map_err(CoreEnvironmentError::MakeSandbox)?
            .into_path();

        debug!(
            "transaction: making temporary environment in {}",
            tempdir.display()
        );
        let mut temp_env = self.writable(&tempdir)?;

        debug!("transaction: updating manifest");
        temp_env.update_manifest(&contents)?;

        debug!("transaction: building environment, ignoring errors (unsafe)");

        if let Err(lock_err) = temp_env.lock(flox) {
            debug!("transaction: lock failed: {:?}", lock_err);
            debug!("transaction: replacing environment");
            self.replace_with(temp_env)?;
            return Ok(Err(lock_err));
        };

        let build_attempt = temp_env.build(flox);

        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;

        match build_attempt {
            Ok(store_path) => Ok(EditResult::new(&old_contents, &contents, Some(store_path))),
            Err(err) => Ok(Err(err)),
        }
    }

    /// Update the inputs of an environment atomically.
    pub fn update(
        &mut self,
        flox: &Flox,
        inputs: Vec<String>,
    ) -> Result<UpdateResult, CoreEnvironmentError> {
        // TODO: double check canonicalization
        let UpdateResult {
            new_lockfile,
            old_lockfile,
            ..
        } = LockedManifestPkgdb::update_manifest(
            flox,
            Some(self.manifest_path()),
            self.lockfile_path(),
            inputs,
        )
        .map_err(CoreEnvironmentError::LockedManifest)?;

        let store_path = self.transact_with_lockfile_contents(
            serde_json::to_string_pretty(&new_lockfile).unwrap(),
            flox,
        )?;

        Ok(UpdateResult {
            new_lockfile,
            old_lockfile,
            store_path: Some(store_path),
        })
    }

    /// Atomically upgrade packages in this environment
    ///
    /// First resolve a new lockfile with upgraded packages using either pkgdb or the catalog client.
    /// Then verify the new lockfile by building the environment.
    /// Finally replace the existing environment with the new, upgraded one.
    pub fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[String],
    ) -> Result<UpgradeResult, CoreEnvironmentError> {
        tracing::debug!(to_upgrade = groups_or_iids.join(","), "upgrading");
        let manifest = self.manifest()?;

        let (lockfile, upgraded) = match manifest {
            TypedManifest::Pkgdb(_) => {
                tracing::debug!("using pkgdb to upgrade");
                let (lockfile, upgraded) = self.upgrade_with_pkgdb(flox, groups_or_iids)?;
                (LockedManifest::Pkgdb(lockfile), upgraded)
            },
            TypedManifest::Catalog(catalog) => {
                Self::ensure_valid_upgrade(groups_or_iids, &catalog)?;
                tracing::debug!("using catalog client to upgrade");
                let client = flox
                    .catalog_client
                    .as_ref()
                    .ok_or(CoreEnvironmentError::CatalogClientMissing)?;

                let (lockfile, upgraded) =
                    self.upgrade_with_catalog_client(client, groups_or_iids, &catalog)?;

                let upgraded = {
                    let mut install_ids = upgraded
                        .into_iter()
                        .map(|(_, pkg)| pkg.install_id.clone())
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>();
                    install_ids.sort();
                    install_ids
                };

                (LockedManifest::Catalog(lockfile), upgraded)
            },
        };

        let store_path =
            self.transact_with_lockfile_contents(serde_json::json!(&lockfile).to_string(), flox)?;

        Ok(UpgradeResult {
            packages: upgraded,
            store_path: Some(store_path),
        })
    }

    fn ensure_valid_upgrade(
        groups_or_iids: &[String],
        manifest: &TypedManifestCatalog,
    ) -> Result<(), CoreEnvironmentError> {
        for id in groups_or_iids {
            tracing::debug!(id, "checking that id is a package or group");
            if id == "toplevel" {
                continue;
            }
            if !manifest.pkg_or_group_found_in_manifest(id) {
                return Err(CoreEnvironmentError::UpgradeFailedCatalog(
                    UpgradeError::PkgNotFound(ManifestError::PkgOrGroupNotFound(id.to_string())),
                ));
            }
        }
        tracing::debug!("checking group membership for requested packages");
        for id in groups_or_iids {
            if manifest.pkg_descriptor_with_id(id).is_none() {
                // We've already checked that the id is a package or group,
                // and if this is None then we know it's a group and therefore
                // we don't need to check what other packages are in the group
                // with this id.
                continue;
            }
            if manifest
                .pkg_belongs_to_non_empty_toplevel_group(id)
                .expect("already checked that package exists")
            {
                return Err(CoreEnvironmentError::UpgradeFailedCatalog(
                    UpgradeError::NonEmptyNamedGroup {
                        pkg: id.clone(),
                        group: "toplevel".to_string(),
                    },
                ));
            }
            if let Some(group) = manifest
                .pkg_belongs_to_non_empty_named_group(id)
                .expect("already checked that package exists")
            {
                return Err(CoreEnvironmentError::UpgradeFailedCatalog(
                    UpgradeError::NonEmptyNamedGroup {
                        pkg: id.clone(),
                        group,
                    },
                ));
            }
        }
        Ok(())
    }

    fn upgrade_with_pkgdb(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[String],
    ) -> Result<(LockedManifestPkgdb, Vec<String>), CoreEnvironmentError> {
        let manifest_path = self.manifest_path();
        let lockfile_path = self.lockfile_path();
        let maybe_lockfile = if lockfile_path.exists() {
            debug!("found existing lockfile: {}", lockfile_path.display());
            Some(lockfile_path)
        } else {
            debug!("no existing lockfile found");
            None
        };
        let mut pkgdb_cmd = Command::new(Path::new(&*PKGDB_BIN));
        pkgdb_cmd
            .args(["manifest", "upgrade"])
            .arg("--ga-registry")
            .arg("--global-manifest")
            .arg(global_manifest_path(flox))
            .arg("--manifest")
            .arg(manifest_path);
        if let Some(lf_path) = maybe_lockfile {
            let canonical_lockfile_path =
                CanonicalPath::new(lf_path).map_err(CoreEnvironmentError::BadLockfilePath)?;
            pkgdb_cmd.arg("--lockfile").arg(canonical_lockfile_path);
        }
        pkgdb_cmd.args(groups_or_iids);

        debug!(
            "upgrading environment with command: {}",
            pkgdb_cmd.display()
        );
        let json: UpgradeResultJSON = serde_json::from_value(
            call_pkgdb(pkgdb_cmd).map_err(CoreEnvironmentError::UpgradeFailedPkgDb)?,
        )
        .map_err(CoreEnvironmentError::ParseUpgradeOutput)?;

        Ok((json.lockfile, json.result.0))
    }

    /// Upgrade the given groups or install ids in the environment using the catalog client.
    /// The environment is upgraded by locking the existing manifest
    /// using [LockedManifestCatalog::lock_manifest] with the existing lockfile as a seed,
    /// where the upgraded packages have been filtered out causing them to be re-resolved.
    fn upgrade_with_catalog_client(
        &mut self,
        client: &impl ClientTrait,
        groups_or_iids: &[String],
        manifest: &TypedManifestCatalog,
    ) -> Result<
        (
            LockedManifestCatalog,
            Vec<(LockedPackageCatalog, LockedPackageCatalog)>,
        ),
        CoreEnvironmentError,
    > {
        tracing::debug!(to_upgrade = groups_or_iids.join(","), "upgrading");
        let existing_lockfile = 'lockfile: {
            let Ok(lockfile_path) = CanonicalPath::new(self.lockfile_path()) else {
                break 'lockfile None;
            };
            let lockfile = LockedManifest::read_from_file(&lockfile_path)
                .map_err(CoreEnvironmentError::LockedManifest)?;
            match lockfile {
                LockedManifest::Catalog(lockfile) => Some(lockfile),
                _ => {
                    // This will be the case when performing a migration
                    debug!(
                        "Found version 1 manifest, but lockfile doesn't match: Ignoring lockfile."
                    );
                    None
                },
            }
        };

        // Record a nested map where you retrieve the locked package
        // via pkgs[install_id][system]
        let previous_packages = if let Some(ref lockfile) = existing_lockfile {
            let mut pkgs_by_id = BTreeMap::new();
            lockfile.packages.iter().for_each(|pkg| {
                let by_system = pkgs_by_id
                    .entry(pkg.install_id.clone())
                    .or_insert(BTreeMap::new());
                by_system.entry(pkg.system.clone()).or_insert(pkg.clone());
            });
            pkgs_by_id
        } else {
            BTreeMap::new()
        };

        // Create a seed lockfile by "unlocking" (i.e. removing the locked entries of)
        // all packages matching the given groups or iids.
        // If no groups or iids are provided, all packages are unlocked.
        let seed_lockfile = if groups_or_iids.is_empty() {
            debug!("no groups or iids provided, unlocking all packages");
            None
        } else {
            existing_lockfile.clone().map(|mut lockfile| {
                lockfile.unlock_packages_by_group_or_iid(groups_or_iids);
                lockfile
            })
        };

        let upgraded_lockfile =
            LockedManifestCatalog::lock_manifest(manifest, seed_lockfile.as_ref(), client)
                .block_on()
                .map_err(CoreEnvironmentError::LockedManifest)?;

        let pkgs_after_upgrade = {
            let mut pkgs_by_id = BTreeMap::new();
            upgraded_lockfile.packages.iter().for_each(|pkg| {
                let by_system = pkgs_by_id
                    .entry(pkg.install_id.clone())
                    .or_insert(BTreeMap::new());
                by_system.entry(pkg.system.clone()).or_insert(pkg.clone());
            });
            pkgs_by_id
        };

        // Iterate over the two sorted maps in lockstep
        let package_diff = previous_packages
            .iter()
            .zip(pkgs_after_upgrade.iter())
            .flat_map(|((_prev_id, prev_map), (_curr_id, curr_map))| {
                let curr_iter = curr_map.iter().map(|(_sys, pkg)| pkg);
                prev_map.iter().map(|(_sys, pkg)| pkg).zip(curr_iter)
            })
            .filter_map(|(prev_pkg, curr_pkg)| {
                if prev_pkg.derivation != curr_pkg.derivation {
                    Some((prev_pkg.clone(), curr_pkg.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let final_lockfile = if package_diff.is_empty() {
            existing_lockfile.unwrap_or(upgraded_lockfile)
        } else {
            upgraded_lockfile
        };

        Ok((final_lockfile, package_diff))
    }

    /// Makes a temporary copy of the environment so modifications to the manifest
    /// can be applied without modifying the original environment.
    fn writable(
        &mut self,
        tempdir: impl AsRef<Path>,
    ) -> Result<CoreEnvironment<ReadWrite>, CoreEnvironmentError> {
        copy_dir_recursive(&self.env_dir, &tempdir.as_ref(), true)
            .map_err(CoreEnvironmentError::MakeTemporaryEnv)?;

        Ok(CoreEnvironment {
            env_dir: tempdir.as_ref().to_path_buf(),
            _state: ReadWrite {},
        })
    }

    /// Replace the contents of this environment (e.g. `.flox/env`)
    /// with that of another environment.
    ///
    /// This will **not** set any out-links to updated versions of the environment.
    fn replace_with(
        &mut self,
        replacement: CoreEnvironment<ReadWrite>,
    ) -> Result<(), CoreEnvironmentError> {
        let transaction_backup = self.env_dir.with_extension("tmp");

        if transaction_backup.exists() {
            debug!(
                "transaction backup exists: {}",
                transaction_backup.display()
            );
            return Err(CoreEnvironmentError::PriorTransaction(transaction_backup));
        }
        debug!(
            "backing up env: from={}, to={}",
            self.env_dir.display(),
            transaction_backup.display()
        );
        fs::rename(&self.env_dir, &transaction_backup)
            .map_err(CoreEnvironmentError::BackupTransaction)?;
        // try to restore the backup if the move fails
        debug!(
            "replacing original env: from={}, to={}",
            replacement.env_dir.display(),
            self.env_dir.display()
        );
        if let Err(err) = copy_dir_recursive(&replacement.env_dir, &self.env_dir, true) {
            debug!(
                "failed to replace env ({}), restoring backup: from={}, to={}",
                err,
                transaction_backup.display(),
                self.env_dir.display(),
            );
            fs::remove_dir_all(&self.env_dir).map_err(CoreEnvironmentError::AbortTransaction)?;
            fs::rename(transaction_backup, &self.env_dir)
                .map_err(CoreEnvironmentError::AbortTransaction)?;
            return Err(CoreEnvironmentError::Move(err));
        }
        debug!("removing backup: path={}", transaction_backup.display());
        fs::remove_dir_all(transaction_backup).map_err(CoreEnvironmentError::RemoveBackup)?;
        Ok(())
    }

    /// Attempt to transactionally replace the manifest contents
    #[must_use = "don't discard the store path of built environments"]
    fn transact_with_manifest_contents(
        &mut self,
        manifest_contents: impl AsRef<str>,
        flox: &Flox,
    ) -> Result<PathBuf, CoreEnvironmentError> {
        // Return an error for deprecated modifications of v0 manifests
        if flox.catalog_client.is_some() {
            let manifest: TypedManifest = toml::from_str(manifest_contents.as_ref())
                .map_err(CoreEnvironmentError::DeserializeManifest)?;
            if let TypedManifest::Pkgdb(_) = manifest {
                Err(CoreEnvironmentError::Version0NotSupported)?;
            }
        }

        let tempdir = tempfile::tempdir_in(&flox.temp_dir)
            .map_err(CoreEnvironmentError::MakeSandbox)?
            .into_path();

        debug!(
            "transaction: making temporary environment in {}",
            tempdir.display()
        );
        let mut temp_env = self.writable(&tempdir)?;

        debug!("transaction: updating manifest");
        temp_env.update_manifest(&manifest_contents)?;

        debug!("transaction: locking environment");
        temp_env.lock(flox)?;

        debug!("transaction: building environment");
        let store_path = temp_env.build(flox)?;

        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;
        Ok(store_path)
    }

    /// Attempt to transactionally replace the lockfile contents
    ///
    /// The lockfile_contents passed to this function must be generated by pkgdb
    /// so that calling `pkgdb manifest lock` with the new lockfile_contents is
    /// idempotent.
    ///
    /// TODO: this is separate from transact_with_manifest_contents because it
    /// shouldn't have to call lock. Currently build calls lock, but we
    /// shouldn't have to lock a second time.
    #[must_use = "don't discard the store path of built environments"]
    fn transact_with_lockfile_contents(
        &mut self,
        lockfile_contents: impl AsRef<str>,
        flox: &Flox,
    ) -> Result<PathBuf, CoreEnvironmentError> {
        let tempdir = tempfile::tempdir_in(&flox.temp_dir)
            .map_err(CoreEnvironmentError::MakeSandbox)?
            .into_path();

        debug!(
            "transaction: making temporary environment in {}",
            tempdir.display()
        );
        let mut temp_env = self.writable(&tempdir)?;

        debug!("transaction: updating lockfile");
        temp_env.update_lockfile(&lockfile_contents)?;

        debug!("transaction: building environment");
        let store_path = temp_env.build(flox)?;

        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;
        Ok(store_path)
    }

    /// Should not be called with
    /// !migration_info.needs_manifest_migration && !migration_info.needs_upgrade
    pub fn migrate_to_v1(
        &mut self,
        flox: &Flox,
        mut migration_info: MigrationInfo,
    ) -> Result<PathBuf, CoreEnvironmentError> {
        let tempdir = tempfile::tempdir_in(&flox.temp_dir)
            .map_err(CoreEnvironmentError::MakeSandbox)?
            .into_path();

        debug!(
            "migration transaction: making temporary environment in {}",
            tempdir.display()
        );
        let mut temp_env = self.writable(&tempdir)?;

        if migration_info.needs_manifest_migration {
            Self::migrate_manifest_contents_to_v1(&mut migration_info.raw_manifest)?;

            debug!("migration transaction: updating manifest");
            temp_env.update_manifest(migration_info.raw_manifest.to_string())?;
        }

        // Lock if there's a v0 manifest, regardless of whether there's a
        // lockfile or what version it is.
        // This could lock an environment that isn't already locked,
        // but particularly for managed and remote environments, we want to keep
        // the lock in sync with the manifest,
        // so we don't want to perform a transaction without locking.
        debug!("migration transaction: locking environment");
        temp_env
            .lock(flox)
            .map_err(|e| CoreEnvironmentError::LockForMigration(Box::new(e)))?;

        debug!("migration transaction: building environment");
        let store_path = temp_env.build(flox)?;

        debug!("migration transaction: replacing environment");
        self.replace_with(temp_env)?;
        Ok(store_path)
    }

    /// Migrate a v0 [RawManifest] to a v1 [RawManifest] by inserting
    /// `version = 1` and moving `hook.script` to `hook.on-activate` if
    /// `hook.on-activate` doesn't already exist.
    ///
    /// `raw_manifest` is expected to be a v0 manifest.
    /// Return an error if the resulting manifest is not a valid v1 manifest.
    /// Note that the modifications are still made even if an error is returned to allow
    /// [Self::migrate_and_edit_unsafe] to use the invalid manifest.
    fn migrate_manifest_contents_to_v1(
        raw_manifest: &mut RawManifest,
    ) -> Result<(), CoreEnvironmentError> {
        // // Insert `version = 1`
        raw_manifest.insert(MANIFEST_VERSION_KEY, toml_edit::value(1));

        // Migrate `hook.script` to `hook.on-activate`
        let hook = raw_manifest.get_mut("hook").and_then(|s| s.as_table_mut());
        if let Some(hook) = hook {
            if hook.get("on-activate").is_none() {
                // Rename `hook.script` to `hook.on-activate`, preserving
                // comments and formatting
                if let Some((script_key, script_item)) = hook.remove_entry("script") {
                    // Unit tests cover this is safe to unwrap
                    let mut on_activate = toml_edit::Key::from_str("on-activate").unwrap();
                    let mut on_activate_key = on_activate.as_mut();
                    let decor = on_activate_key.leaf_decor_mut();
                    *decor = script_key.leaf_decor().clone();
                    let dotted_decor = on_activate_key.dotted_decor_mut();
                    *dotted_decor = script_key.dotted_decor().clone();
                    // Does not preserve order of hooks,
                    // but we only have one field in the hook section.
                    hook.insert_formatted(&on_activate, script_item);
                }
            }
        }

        // Make sure it parses
        raw_manifest
            .to_typed()
            .map_err(CoreEnvironmentError::MigrateManifest)?;
        Ok(())
    }

    /// Replace manifest with provided `contents` and perform migration in a
    /// single transaction
    pub fn migrate_and_edit_unsafe(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<Result<PathBuf, CoreEnvironmentError>, CoreEnvironmentError> {
        let tempdir = tempfile::tempdir_in(&flox.temp_dir)
            .map_err(CoreEnvironmentError::MakeSandbox)?
            .into_path();

        debug!(
            "migration transaction: making temporary environment in {}",
            tempdir.display()
        );
        let mut temp_env = self.writable(&tempdir)?;

        let mut raw_manifest = RawManifest::from_str(&contents)
            .map_err(|e| CoreEnvironmentError::ModifyToml(TomlEditError::ParseManifest(e)))?;

        let migrate_result = Self::migrate_manifest_contents_to_v1(&mut raw_manifest);

        debug!("migration transaction: updating manifest");
        temp_env.update_manifest(raw_manifest.to_string())?;

        // Check if the manifest is valid after updating it, because we want to
        // update it no matter what.
        if let Err(migrate_error) = migrate_result {
            debug!(
                "migration transaction: migration failed: {:?}",
                migrate_error
            );
            debug!("migration transaction: replacing environment");
            self.replace_with(temp_env)?;
            return Ok(Err(migrate_error));
        }

        if let Err(lock_err) = temp_env.lock(flox) {
            debug!("migration transaction: lock failed: {:?}", lock_err);
            debug!("migration transaction: replacing environment");
            self.replace_with(temp_env)?;
            return Ok(Err(lock_err));
        };

        let build_attempt = temp_env.build(flox);

        debug!("migration transaction: replacing environment");
        self.replace_with(temp_env)?;

        match build_attempt {
            Ok(store_path) => Ok(Ok(store_path)),
            Err(err) => Ok(Err(err)),
        }
    }
}

/// A writable view of an environment directory
///
/// Typically within a temporary directory created by [CoreEnvironment::writable].
/// This is not public to enforce that environments are only edited atomically.
impl CoreEnvironment<ReadWrite> {
    /// Updates the environment manifest with the provided contents
    fn update_manifest(&mut self, contents: impl AsRef<str>) -> Result<(), CoreEnvironmentError> {
        debug!("writing new manifest to {}", self.manifest_path().display());
        let mut manifest_file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(self.manifest_path())
            .map_err(CoreEnvironmentError::OpenManifest)?;
        manifest_file
            .write_all(contents.as_ref().as_bytes())
            .map_err(CoreEnvironmentError::UpdateManifest)?;
        Ok(())
    }

    /// Updates the environment lockfile with the provided contents
    fn update_lockfile(&mut self, contents: impl AsRef<str>) -> Result<(), CoreEnvironmentError> {
        debug!("writing lockfile to {}", self.lockfile_path().display());
        std::fs::write(self.lockfile_path(), contents.as_ref())
            .map_err(CoreEnvironmentError::WriteLockfile)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditResult {
    /// The manifest was not modified.
    Unchanged,
    /// The manifest was modified, and the user needs to re-activate it.
    ReActivateRequired { store_path: Option<PathBuf> },
    /// The manifest was modified, but the user does not need to re-activate it.
    Success { store_path: Option<PathBuf> },
}

impl EditResult {
    pub fn new(
        old_manifest: &str,
        new_manifest: &str,
        store_path: Option<PathBuf>,
    ) -> Result<Self, CoreEnvironmentError> {
        if old_manifest == new_manifest {
            Ok(Self::Unchanged)
        } else {
            // todo: use a single toml crate (toml_edit already implements serde traits)
            // TODO: use different error variants, users _can_ fix errors in the _new_ manifest
            //       but they _can't_ fix errors in the _old_ manifest
            let old_manifest: TypedManifest =
                toml::from_str(old_manifest).map_err(CoreEnvironmentError::DeserializeManifest)?;
            let new_manifest: TypedManifest =
                toml::from_str(new_manifest).map_err(CoreEnvironmentError::DeserializeManifest)?;

            match (&old_manifest, &new_manifest) {
                (TypedManifest::Pkgdb(old), TypedManifest::Pkgdb(new)) => {
                    if old.hook != new.hook || old.vars != new.vars || old.profile != new.profile {
                        Ok(Self::ReActivateRequired { store_path })
                    } else {
                        Ok(Self::Success { store_path })
                    }
                },
                (TypedManifest::Catalog(old), TypedManifest::Catalog(new)) => {
                    if old.hook != new.hook || old.vars != new.vars || old.profile != new.profile {
                        Ok(Self::ReActivateRequired { store_path })
                    } else {
                        Ok(Self::Success { store_path })
                    }
                },
                (TypedManifest::Catalog(catalog), TypedManifest::Pkgdb(pkgdb))
                | (TypedManifest::Pkgdb(pkgdb), TypedManifest::Catalog(catalog)) => {
                    if toml::Value::try_from(&pkgdb.hook) != toml::Value::try_from(&catalog.hook)
                        || toml::Value::try_from(&pkgdb.vars)
                            != toml::Value::try_from(&catalog.vars)
                        || toml::Value::try_from(&pkgdb.profile)
                            != toml::Value::try_from(&catalog.profile)
                    {
                        Ok(Self::ReActivateRequired { store_path })
                    } else {
                        Ok(Self::Success { store_path })
                    }
                },
            }
        }
    }

    pub fn store_path(&self) -> Option<PathBuf> {
        match self {
            EditResult::Unchanged => None,
            EditResult::ReActivateRequired { store_path } => store_path.clone(),
            EditResult::Success { store_path } => store_path.clone(),
        }
    }
}

#[derive(Debug, Error)]
pub enum CoreEnvironmentError {
    // region: immutable manifest errors
    #[error("could not modify manifest")]
    ModifyToml(#[source] TomlEditError),
    #[error("could not deserialize manifest")]
    DeserializeManifest(#[source] toml::de::Error),
    // endregion

    // region: transaction errors
    #[error("could not make temporary directory for transaction")]
    MakeSandbox(#[source] std::io::Error),

    #[error("couldn't write new lockfile contents")]
    WriteLockfile(#[source] std::io::Error),

    #[error("could not make temporary copy of environment")]
    MakeTemporaryEnv(#[source] std::io::Error),
    /// Thrown when a .flox/env.tmp directory already exists
    #[error("prior transaction in progress -- delete {0} to discard")]
    PriorTransaction(PathBuf),
    #[error("could not create backup for transaction")]
    BackupTransaction(#[source] std::io::Error),
    #[error("Failed to abort transaction; backup could not be moved back into place")]
    AbortTransaction(#[source] std::io::Error),
    #[error("Failed to move modified environment into place")]
    Move(#[source] std::io::Error),
    #[error("Failed to remove transaction backup")]
    RemoveBackup(#[source] std::io::Error),

    // endregion

    // region: mutable manifest errors
    #[error("could not open manifest")]
    OpenManifest(#[source] std::io::Error),
    #[error("could not write manifest")]
    UpdateManifest(#[source] std::io::Error),
    // endregion

    // region: pkgdb manifest errors
    #[error(transparent)]
    LockedManifest(LockedManifestError),

    #[error(transparent)]
    BadLockfilePath(CanonicalizeError),

    // todo: refactor upgrade to use `LockedManifest`
    #[error("unexpected output from pkgdb upgrade")]
    ParseUpgradeOutput(#[source] serde_json::Error),
    #[error("failed to upgrade environment")]
    UpgradeFailedPkgDb(#[source] CallPkgDbError),
    #[error("failed to upgrade environment")]
    UpgradeFailedCatalog(#[source] UpgradeError),
    // endregion

    // endregion
    #[error("unsupported system to build container: {0}")]
    ContainerizeUnsupportedSystem(String),

    #[error("Could not process catalog manifest without a catalog client")]
    CatalogClientMissing,

    #[error("could not automatically migrate manifest to version 1")]
    MigrateManifest(#[source] toml_edit::de::Error),

    #[error("failed to create version 1 lock")]
    LockForMigration(#[source] Box<CoreEnvironmentError>),

    #[error(
        "Modifying version 0 manifests is no longer supported.\nSet 'version = 1' in the manifest."
    )]
    Version0NotSupported,
}

impl CoreEnvironmentError {
    pub fn is_incompatible_system_error(&self) -> bool {
        let is_pkgdb_incompatible_system_error = matches!(
            self,
            CoreEnvironmentError::LockedManifest(LockedManifestError::BuildEnv(
                CallPkgDbError::PkgDbError(PkgDbError {
                    exit_code: error_codes::LOCKFILE_INCOMPATIBLE_SYSTEM,
                    ..
                })
            ))
        );
        let is_catalog_incompatible_system_error = matches!(
            self,
            CoreEnvironmentError::LockedManifest(LockedManifestError::ResolutionFailed(failures))
             if failures.0.iter().any(|f| matches!(f, ResolutionFailure::PackageUnavailableOnSomeSystems { .. })));
        is_catalog_incompatible_system_error || is_pkgdb_incompatible_system_error
    }

    pub fn is_incompatible_package_error(&self) -> bool {
        #[allow(clippy::match_like_matches_macro)] // rustfmt can't handle this as a match!
        match self.pkgdb_exit_code() {
            Some(exit_code)
                if [
                    error_codes::PACKAGE_BUILD_FAILURE,
                    error_codes::PACKAGE_EVAL_FAILURE,
                    error_codes::PACKAGE_EVAL_INCOMPATIBLE_SYSTEM,
                ]
                .contains(exit_code) =>
            {
                true
            },
            _ => false,
        }
    }

    /// If the error contains a PkgDbError with an exit_code, return it.
    /// Otherwise return None.
    pub fn pkgdb_exit_code(&self) -> Option<&u64> {
        match self {
            CoreEnvironmentError::LockedManifest(LockedManifestError::BuildEnv(
                CallPkgDbError::PkgDbError(PkgDbError { exit_code, .. }),
            )) => Some(exit_code),
            _ => None,
        }
    }
}

pub mod test_helpers {
    use indoc::indoc;

    use super::*;
    use crate::flox::Flox;

    pub const MANIFEST_V0_FIELDS: &str = indoc! {r#"
        [options]
        semver.prefer-pre-releases = true
        "#};
    // TODO: add version = 1 to this manifest
    #[cfg(target_os = "macos")]
    pub const MANIFEST_INCOMPATIBLE_SYSTEM: &str = indoc! {r#"
        [options]
        systems = ["x86_64-linux"]
        "#};
    #[cfg(target_os = "macos")]
    pub const MANIFEST_INCOMPATIBLE_SYSTEM_V0_FIELDS: &str = indoc! {r#"
        [options]
        systems = ["x86_64-linux"]
        semver.prefer-pre-releases = true
        "#};
    #[cfg(target_os = "macos")]
    pub const MANIFEST_INCOMPATIBLE_SYSTEM_V1: &str = indoc! {r#"
        version = 1
        [options]
        systems = ["x86_64-linux"]
        "#};

    #[cfg(target_os = "linux")]
    pub const MANIFEST_INCOMPATIBLE_SYSTEM: &str = indoc! {r#"
        [options]
        systems = ["aarch64-darwin"]
        "#};
    #[cfg(target_os = "linux")]
    pub const MANIFEST_INCOMPATIBLE_SYSTEM_V0_FIELDS: &str = indoc! {r#"
        [options]
        systems = ["aarch64-darwin"]
        semver.prefer-pre-releases = true
        "#};
    #[cfg(target_os = "linux")]
    pub const MANIFEST_INCOMPATIBLE_SYSTEM_V1: &str = indoc! {r#"
        version = 1
        [options]
        systems = ["aarch64-darwin"]
        "#};

    pub fn new_core_environment(flox: &Flox, contents: &str) -> CoreEnvironment {
        let env_path = tempfile::tempdir_in(&flox.temp_dir).unwrap().into_path();
        fs::write(env_path.join(MANIFEST_FILENAME), contents).unwrap();

        CoreEnvironment::new(&env_path)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::str::FromStr;

    use catalog::Client;
    use catalog_api_v1::types::{ResolvedPackageDescriptor, SystemEnum};
    use chrono::{DateTime, Utc};
    use indoc::{formatdoc, indoc};
    use pretty_assertions::assert_eq;
    use serial_test::serial;
    use tempfile::{tempdir_in, TempDir};
    use tests::test_helpers::MANIFEST_INCOMPATIBLE_SYSTEM;

    use self::catalog::{CatalogPage, MockClient, ResolvedPackageGroup};
    use self::test_helpers::new_core_environment;
    use super::*;
    use crate::data::Version;
    use crate::flox::test_helpers::{
        flox_instance,
        flox_instance_with_global_lock,
        flox_instance_with_optional_floxhub_and_client,
    };
    use crate::models::lockfile::test_helpers::fake_package;
    use crate::models::lockfile::ResolutionFailures;
    use crate::models::manifest::{RawManifest, DEFAULT_GROUP_NAME};
    use crate::models::{lockfile, manifest};

    /// Create a CoreEnvironment with an empty manifest
    ///
    /// This calls flox_instance_with_global_lock(),
    /// so the resulting environment can be built without incurring a pkgdb scrape.
    fn empty_core_environment() -> (CoreEnvironment, Flox, TempDir) {
        let (flox, tempdir) = flox_instance_with_global_lock();

        (new_core_environment(&flox, ""), flox, tempdir)
    }

    /// Check that `edit` updates the manifest and creates a lockfile
    #[test]
    #[serial]
    #[cfg(feature = "impure-unit-tests")]
    fn edit_env_creates_manifest_and_lockfile() {
        let (flox, tempdir) = flox_instance_with_global_lock();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        fs::write(env_path.path().join(MANIFEST_FILENAME), "").unwrap();

        let mut env_view = CoreEnvironment::new(&env_path);

        let new_env_str = r#"
        [install]
        hello = {}
        "#;

        env_view.edit(&flox, new_env_str.to_string()).unwrap();

        assert_eq!(env_view.manifest_content().unwrap(), new_env_str);
        assert!(env_view.env_dir.join(LOCKFILE_FILENAME).exists());
    }

    /// A no-op with edit returns EditResult::Unchanged
    #[test]
    #[serial]
    fn edit_no_op_returns_unchanged() {
        let (mut env_view, flox, _temp_dir_handle) = empty_core_environment();

        let result = env_view.edit(&flox, "".to_string()).unwrap();

        assert!(matches!(result, EditResult::Unchanged));
    }

    /// Trying to build a manifest with a system other than the current one
    /// results in an error that is_incompatible_system_error()
    #[test]
    #[serial]
    fn build_incompatible_system() {
        let (mut env_view, flox, _temp_dir_handle) = empty_core_environment();
        let mut temp_env = env_view
            .writable(tempdir_in(&flox.temp_dir).unwrap().into_path())
            .unwrap();
        temp_env
            .update_manifest(MANIFEST_INCOMPATIBLE_SYSTEM)
            .unwrap();
        temp_env.lock(&flox).unwrap();
        env_view.replace_with(temp_env).unwrap();

        let result = env_view.build(&flox).unwrap_err();

        assert!(result.is_incompatible_system_error());
    }

    /// Trying to build a manifest with a package that is incompatible with the current system
    /// results in an error that is_incompatible_package_error()
    #[test]
    #[serial]
    fn build_incompatible_package() {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        let manifest_contents = formatdoc! {r#"
        [install]
        glibc.pkg-path = "glibc"

        [options]
        systems = ["aarch64-darwin"]
        "#};

        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        let manifest_contents = formatdoc! {r#"
        [install]
        glibc.pkg-path = "glibc"

        [options]
        systems = ["x86_64-darwin"]
        "#};

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let manifest_contents = formatdoc! {r#"
        [install]
        ps.pkg-path = "darwin.ps"

        [options]
        systems = ["x86_64-linux"]
        "#};

        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        let manifest_contents = formatdoc! {r#"
        [install]
        ps.pkg-path = "darwin.ps"

        [options]
        systems = ["aarch64-linux"]
        "#};

        let (mut env_view, flox, _temp_dir_handle) = empty_core_environment();
        let mut temp_env = env_view
            .writable(tempdir_in(&flox.temp_dir).unwrap().into_path())
            .unwrap();
        temp_env.update_manifest(&manifest_contents).unwrap();
        temp_env.lock(&flox).unwrap();
        env_view.replace_with(temp_env).unwrap();

        let result = env_view.build(&flox).unwrap_err();

        assert!(result.is_incompatible_package_error());
    }

    /// Trying to build a manifest with an insecure package results in an error
    /// that is_incompatible_package_error()
    #[test]
    #[serial]
    fn build_insecure_package() {
        let manifest_content = indoc! {r#"
            [install]
            python2.pkg-path = "python2"
            "#
        };
        let (mut env_view, flox, _temp_dir_handle) = empty_core_environment();
        let mut temp_env = env_view
            .writable(tempdir_in(&flox.temp_dir).unwrap().into_path())
            .unwrap();
        temp_env.update_manifest(manifest_content).unwrap();
        temp_env.lock(&flox).unwrap();
        env_view.replace_with(temp_env).unwrap();

        let result = env_view.build(&flox).unwrap_err();

        assert!(result.is_incompatible_package_error());
    }

    /// Installing hello with edit returns EditResult::Success
    #[test]
    #[serial]
    fn edit_adding_package_returns_success() {
        let (mut env_view, flox, _temp_dir_handle) = empty_core_environment();

        let new_env_str = r#"
        [install]
        hello = {}
        "#;

        let result = env_view.edit(&flox, new_env_str.to_string()).unwrap();

        assert!(matches!(result, EditResult::Success { store_path: _ }));
    }

    /// Adding a hook with edit returns EditResult::ReActivateRequired
    #[test]
    #[serial]
    fn edit_adding_hook_returns_re_activate_required() {
        let (mut env_view, flox, _temp_dir_handle) = empty_core_environment();

        let new_env_str = r#"
        [hook]
        on-activate = ""
        "#;

        let result = env_view.edit(&flox, new_env_str.to_string()).unwrap();

        assert!(matches!(result, EditResult::ReActivateRequired {
            store_path: _
        }));
    }

    #[test]
    fn locking_of_v1_manifest_requires_catalog_client() {
        let (mut env_view, mut flox, _temp_dir_handle) = empty_core_environment();
        flox.catalog_client = None;

        fs::write(env_view.manifest_path(), r#"version = 1"#).unwrap();

        let err = env_view
            .lock(&flox)
            .expect_err("should fail to lock v1 lockfile with pkgdb");

        assert!(matches!(err, CoreEnvironmentError::CatalogClientMissing));

        let mut mock_client = MockClient::new(None::<&str>).unwrap();
        mock_client.push_resolve_response(vec![]);
        flox.catalog_client = Option::Some(mock_client.into());

        env_view
            .lock(&flox)
            .expect("lock should succeed with catalog client");
    }

    #[test]
    fn upgrade_with_catalog_client_requires_catalog_client() {
        // flox already has a catalog client
        let (mut env_view, mut flox, _temp_dir_handle) = empty_core_environment();
        fs::write(env_view.manifest_path(), r#"version = 1"#).unwrap();

        flox.catalog_client = None;
        let err = env_view
            .upgrade(&flox, &[])
            .expect_err("upgrade of v1 manifest should fail without client");

        assert!(matches!(err, CoreEnvironmentError::CatalogClientMissing));

        let mut mock_client = MockClient::new(None::<&str>).unwrap();
        mock_client.push_resolve_response(vec![]);
        flox.catalog_client = Option::Some(mock_client.into());
        env_view
            .upgrade(&flox, &[])
            .expect("upgrade should succeed with catalog client");
    }

    /// Check that with an empty list of packages to upgrade, all packages are upgraded
    // TODO: add fixtures for resolve mocks if we add more of these tests
    #[test]
    fn upgrade_with_empty_list_upgrades_all() {
        let (mut env_view, _flox, _temp_dir_handle) = empty_core_environment();

        let mut manifest = manifest::test::empty_catalog_manifest();
        let (foo_iid, foo_descriptor, foo_locked) = fake_package("foo", None);
        manifest.install.insert(foo_iid.clone(), foo_descriptor);
        let lockfile = lockfile::LockedManifestCatalog {
            version: Version,
            packages: vec![foo_locked.clone()],
            manifest: manifest.clone(),
        };

        let lockfile_str = serde_json::to_string_pretty(&lockfile).unwrap();

        fs::write(env_view.lockfile_path(), lockfile_str).unwrap();

        let mut mock_client = MockClient::new(None::<&str>).unwrap();
        mock_client.push_resolve_response(vec![ResolvedPackageGroup {
            name: DEFAULT_GROUP_NAME.to_string(),
            page: Some(CatalogPage {
                packages: Some(vec![ResolvedPackageDescriptor {
                    attr_path: "foo".to_string(),
                    broken: Some(false),
                    derivation: "new derivation".to_string(),
                    description: Some("description".to_string()),
                    install_id: foo_iid.clone(),
                    license: None,
                    locked_url: "locked-url".to_string(),
                    name: "foo".to_string(),
                    outputs: vec![],
                    outputs_to_install: None,
                    pname: "foo".to_string(),
                    rev: "rev".to_string(),
                    rev_count: 42,
                    rev_date: DateTime::<Utc>::MIN_UTC,
                    scrape_date: DateTime::<Utc>::MIN_UTC,
                    stabilities: None,
                    unfree: None,
                    version: "1.0".to_string(),
                    system: SystemEnum::Aarch64Darwin,
                }]),
                msgs: vec![],
                page: 1,
                url: "url".to_string(),
                complete: true,
            }),
            msgs: vec![],
        }]);

        let (_, upgraded_packages) = env_view
            .upgrade_with_catalog_client(&mock_client, &[], &manifest)
            .unwrap();

        assert!(upgraded_packages.len() == 1);
    }

    /// replacing an environment should fail if a backup exists
    #[test]
    fn detects_existing_backup() {
        let (_flox, tempdir) = flox_instance();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        let sandbox_path = tempfile::tempdir_in(&tempdir).unwrap();
        fs::create_dir(env_path.path().with_extension("tmp")).unwrap();

        let mut env_view = CoreEnvironment::new(&env_path);
        let temp_env = env_view.writable(&sandbox_path).unwrap();

        let err = env_view
            .replace_with(temp_env)
            .expect_err("Should fail if backup exists");

        assert!(matches!(err, CoreEnvironmentError::PriorTransaction(_)));
    }

    /// creating backup should fail if env is readonly
    #[test]
    #[ignore = "On Ubuntu github runners this moving a read only directory succeeds.
        thread 'models::environment::core_environment::tests::fails_to_create_backup' panicked at 'Should fail to create backup: dir is readonly: 40555: ()'"]
    fn fails_to_create_backup() {
        let (_flox, tempdir) = flox_instance();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        let sandbox_path = tempfile::tempdir_in(&tempdir).unwrap();

        let mut env_path_permissions = fs::metadata(env_path.path()).unwrap().permissions();
        env_path_permissions.set_readonly(true);

        // force fail by setting dir readonly
        fs::set_permissions(&env_path, env_path_permissions.clone()).unwrap();

        let mut env_view = CoreEnvironment::new(&env_path);
        let temp_env = env_view.writable(&sandbox_path).unwrap();

        let err = env_view.replace_with(temp_env).expect_err(&format!(
            "Should fail to create backup: dir is readonly: {:o}",
            env_path_permissions.mode()
        ));

        assert!(
            matches!(err, CoreEnvironmentError::BackupTransaction(err) if err.kind() == std::io::ErrorKind::PermissionDenied)
        );
    }

    /// linking an environment should set a gc-root
    #[test]
    #[serial]
    #[cfg(feature = "impure-unit-tests")]
    fn build_flox_environment_and_links() {
        let (flox, tempdir) = flox_instance_with_global_lock();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        fs::write(
            env_path.path().join(MANIFEST_FILENAME),
            "
        [install]
        hello = {}
        ",
        )
        .unwrap();

        let mut env_view = CoreEnvironment::new(&env_path);

        env_view.lock(&flox).expect("locking should succeed");
        env_view.build(&flox).expect("build should succeed");
        env_view
            .link(&flox, env_path.path().with_extension("out-link"), &None)
            .expect("link should succeed");

        // very rudimentary check that the environment manifest built correctly
        // and linked to the out-link.
        assert!(env_path
            .path()
            .with_extension("out-link")
            .join("bin/hello")
            .exists());
    }

    #[test]
    fn migrate_to_v1_error_for_dropped_field() {
        let (flox, _temp_dir_handle) = flox_instance();
        let contents = indoc! {r#"
            [options]
            semver.prefer-pre-releases = true
            "#};

        let mut environment = new_core_environment(&flox, contents);

        let raw_manifest = RawManifest::from_str(contents).unwrap();

        let err = environment
            .migrate_to_v1(&flox, MigrationInfo {
                raw_manifest,
                needs_manifest_migration: true,
                needs_upgrade: false,
            })
            .unwrap_err();

        if let CoreEnvironmentError::MigrateManifest(e) = err {
            assert!(e.message().contains("unknown field `prefer-pre-releases`"));
        } else {
            panic!("expected MigrateManifest error");
        }
    }

    #[test]
    fn migrate_to_v1_error_for_locking() {
        let (flox_pkgdb, _temp_dir_handle) = flox_instance_with_global_lock();
        let contents = indoc! {r#"
            [install]
            glibc.pkg-path = "glibc"

            [options]
            systems = [ "x86_64-linux", "aarch64-darwin" ]
            "#};

        let mut environment = new_core_environment(&flox_pkgdb, contents);
        // The v0 lockfile should get ignored,
        // but create it just to keep this more realistic
        environment.lock(&flox_pkgdb).unwrap();

        let (mut flox_catalog, _temp_dir_handle) =
            flox_instance_with_optional_floxhub_and_client(None, true);
        if let Some(Client::Mock(ref mut client)) = flox_catalog.catalog_client {
            client.clear_and_load_responses_from_file("resolve/glibc_incompatible.json");
        } else {
            panic!("expected Mock client")
        };

        let raw_manifest = RawManifest::from_str(contents).unwrap();

        let err = environment
            .migrate_to_v1(&flox_catalog, MigrationInfo {
                raw_manifest,
                needs_manifest_migration: true,
                needs_upgrade: true,
            })
            .unwrap_err();

        if let CoreEnvironmentError::LockForMigration(e) = err {
            if let CoreEnvironmentError::LockedManifest(LockedManifestError::ResolutionFailed(
                ResolutionFailures(failures),
            )) = *e
            {
                assert!(failures.len() == 1);
                assert_eq!(
                    failures[0],
                    ResolutionFailure::PackageUnavailableOnSomeSystems {
                        install_id: "glibc".to_string(),
                        attr_path: "glibc".to_string(),
                        invalid_systems: vec!["aarch64-darwin".to_string()],
                        valid_systems: vec![
                            "aarch64-linux".to_string(),
                            "x86_64-linux".to_string()
                        ],
                    }
                );
            } else {
                panic!("expected ResolutionFailures")
            }
        } else {
            panic!("expected LockForMigration error");
        }
    }

    /// [CoreEnvironment::migrate_manifest_contents_to_v1] migrates a manifest
    /// with `script` in a `[hook]` table correctly, maintaining comments and
    /// formatting.
    #[test]
    fn migrate_script_hook_table() {
        let contents = formatdoc! {r#"
            [vars]
            foo = "bar"

            # comment 1
            [hook] # comment 2
            # comment 3
             script = "echo hello" # comment 4
            # comment 5

            [options]
            "#};
        let mut raw_manifest = RawManifest::from_str(&contents).unwrap();
        CoreEnvironment::migrate_manifest_contents_to_v1(&mut raw_manifest).unwrap();
        assert_eq!(raw_manifest.to_string(), formatdoc! {r#"
                version = 1
                [vars]
                foo = "bar"

                # comment 1
                [hook] # comment 2
                # comment 3
                 on-activate = "echo hello" # comment 4
                # comment 5

                [options]
                "#
        });
    }

    /// [CoreEnvironment::migrate_manifest_contents_to_v1] migrates a manifest
    /// with hook.script as a dotted key correctly, maintaining comments and
    /// formatting.
    #[test]
    fn migrate_script_hook_dotted_decor() {
        let contents = formatdoc! {r#"
            vars.foo = "bar"

            # comment 1
            hook . script = "echo hello" # comment 2
            # comment 3

            options.allow.unfree = false
            "#};
        let mut raw_manifest = RawManifest::from_str(&contents).unwrap();
        CoreEnvironment::migrate_manifest_contents_to_v1(&mut raw_manifest).unwrap();
        assert_eq!(raw_manifest.to_string(), formatdoc! {r#"
                vars.foo = "bar"

                # comment 1
                hook . on-activate = "echo hello" # comment 2
                # comment 3

                options.allow.unfree = false
                version = 1
                "#
        });
    }

    /// If a manifest contains both `hook.script` and `hook.on-activate`,
    /// [CoreEnvironment::migrate_manifest_contents_to_v1] returns an error.
    #[test]
    fn migrate_script_skip_for_on_activate() {
        let contents = formatdoc! {r#"
            [hook]
            script = "echo foo"
            on-activate = "echo bar"
            "#};
        let mut raw_manifest = RawManifest::from_str(&contents).unwrap();
        let err = CoreEnvironment::migrate_manifest_contents_to_v1(&mut raw_manifest).unwrap_err();
        assert_eq!(raw_manifest.to_string(), formatdoc! {r#"
                version = 1
                [hook]
                script = "echo foo"
                on-activate = "echo bar"
                "#
        });
        if let CoreEnvironmentError::MigrateManifest(e) = err {
            assert!(e.message().contains("unknown field `script`"));
        } else {
            panic!("expected MigrateManifest error");
        }
    }

    /// Even if a manifest fails validation, it is still modified by
    /// [CoreEnvironment::migrate_manifest_contents_to_v1].
    #[test]
    fn migrate_script_modifies_on_error() {
        let contents = formatdoc! {r#"
            [hook]
            script = "echo hello"
            on-activate = "echo hello"
            "#};
        let mut raw_manifest = RawManifest::from_str(&contents).unwrap();
        assert!(raw_manifest.get("version").is_none());
        CoreEnvironment::migrate_manifest_contents_to_v1(&mut raw_manifest).unwrap_err();
        assert_eq!(
            raw_manifest.get("version").unwrap().as_integer().unwrap(),
            1
        );
    }
}
