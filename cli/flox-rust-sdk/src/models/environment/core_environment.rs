use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use itertools::Itertools;
use pollster::FutureExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

use super::fetcher::IncludeFetcher;
use super::{
    CanonicalizeError,
    EnvironmentError,
    InstallationAttempt,
    LOCKFILE_FILENAME,
    MANIFEST_FILENAME,
    UninstallationAttempt,
    UpgradeError,
    copy_dir_recursive,
};
use crate::data::CanonicalPath;
use crate::flox::Flox;
use crate::models::lockfile::{
    LockResult,
    LockedInclude,
    LockedPackage,
    Lockfile,
    ResolutionFailure,
    ResolveError,
};
use crate::models::manifest::raw::{
    PackageToInstall,
    TomlEditError,
    insert_packages,
    remove_packages,
};
use crate::models::manifest::typed::{Inner, Manifest, ManifestError, ManifestPackageDescriptor};
use crate::providers::buildenv::{
    BuildEnv,
    BuildEnvError,
    BuildEnvNix,
    BuildEnvOutputs,
    BuiltStorePath,
};
use crate::providers::services::{ServiceError, maybe_make_service_config_file};

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
    /// Includes may be relative to a directory completely unrelated to this
    /// CoreEnvironment's env_dir,
    /// or relative directories may not be allowed as is the case for remote
    /// environments.
    /// The fetcher keeps track of this information.
    include_fetcher: IncludeFetcher,
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
    pub fn manifest_contents(&self) -> Result<String, CoreEnvironmentError> {
        fs::read_to_string(self.manifest_path()).map_err(CoreEnvironmentError::OpenManifest)
    }

    /// Return the contents of the lockfile or None if it doesn't exist
    pub fn existing_lockfile_contents(&self) -> Result<Option<String>, CoreEnvironmentError> {
        let lockfile_path = self.lockfile_path();
        if let Ok(lockfile_path) = CanonicalPath::new(lockfile_path) {
            Ok(Some(
                fs::read_to_string(lockfile_path).map_err(CoreEnvironmentError::ReadLockfile)?,
            ))
        } else {
            Ok(None)
        }
    }

    /// Return a [LockedManifest] if the lockfile exists,
    /// otherwise return None
    pub fn existing_lockfile(&self) -> Result<Option<Lockfile>, CoreEnvironmentError> {
        let lockfile_path = self.lockfile_path();
        if let Ok(lockfile_path) = CanonicalPath::new(lockfile_path) {
            Ok(Some(Lockfile::read_from_file(&lockfile_path)?))
        } else {
            Ok(None)
        }
    }

    pub fn manifest(&self) -> Result<Manifest, CoreEnvironmentError> {
        toml::from_str(&self.manifest_contents()?)
            .map_err(CoreEnvironmentError::DeserializeManifest)
    }

    /// Return a [LockedManifest] if the environment is already locked and has
    /// the same manifest contents as the manifest, otherwise return None.
    /// Note that the manifest could have whitespace or comment differences from
    /// the lockfile.
    pub fn lockfile_if_up_to_date(&self) -> Result<Option<Lockfile>, CoreEnvironmentError> {
        let lockfile_path = self.lockfile_path();

        let Ok(lockfile_path) = CanonicalPath::new(lockfile_path) else {
            return Ok(None);
        };

        let manifest: Manifest = toml::from_str(&self.manifest_contents()?)
            .map_err(CoreEnvironmentError::DeserializeManifest)?;
        let lockfile = Lockfile::read_from_file(&lockfile_path)?;

        // Check if the manifest embedded in the lockfile and the manifest
        // itself have the same contents
        let already_locked = &manifest == lockfile.user_manifest();

        if already_locked {
            Ok(Some(lockfile))
        } else {
            Ok(None)
        }
    }

    /// Lock the environment if it isn't already locked.
    ///
    /// This might be a slight optimization as compared to calling [Self::lock],
    /// but [Self::lock] skips re-locking already locked packages,
    /// so it probably doesn't make much of a difference.
    /// The real point of this method is letting us skip locking for an already
    /// locked pkgdb manifest,
    /// since pkgdb manifests can no longer be locked.
    ///
    /// TODO: consider removing this
    pub fn ensure_locked(&mut self, flox: &Flox) -> Result<LockResult, EnvironmentError> {
        match self.lockfile_if_up_to_date()? {
            Some(lock) => Ok(LockResult::Unchanged(lock)),
            None => self.lock(flox),
        }
    }

    /// Lock the environment.
    ///
    /// Use a catalog client to lock the environment,
    /// and write the lockfile to disk if its contents have changed.
    ///
    /// If the lock should happen conditionally, use [Self::ensure_locked],
    /// or implement the condition in the caller.
    ///
    /// Technically this does write to disk as a side effect for now.
    /// It's included in the [ReadOnly] struct for ergonomic reasons
    /// and because it doesn't modify the manifest.
    pub fn lock(&mut self, flox: &Flox) -> Result<LockResult, EnvironmentError> {
        let manifest = self.manifest()?;
        let existing_lockfile_contents = self.existing_lockfile_contents()?;
        let existing_lockfile = existing_lockfile_contents
            .as_deref()
            .map(Lockfile::from_str)
            .transpose()?;

        // If a lockfile exists, it is used as a base.
        let lockfile = Lockfile::lock_manifest(
            flox,
            &manifest,
            existing_lockfile.as_ref(),
            &self.include_fetcher,
        )
        .block_on()?;

        let lockfile_contents =
            serde_json::to_string_pretty(&lockfile).expect("lockfile structure is valid json");

        let environment_lockfile_path = self.lockfile_path();

        if Some(&lockfile_contents) == existing_lockfile_contents.as_ref() {
            debug!(
                ?environment_lockfile_path,
                "lockfile is up to date, skipping write"
            );
            return Ok(LockResult::Unchanged(lockfile));
        }

        // Write the lockfile to disk
        debug!(
            ?environment_lockfile_path,
            "generated lockfile, writing to disk",
        );

        let mut temp_lockfile = tempfile::NamedTempFile::new_in(&self.env_dir)
            .map_err(CoreEnvironmentError::WriteLockfile)?;

        temp_lockfile
            .write_all(lockfile_contents.as_bytes())
            .map_err(CoreEnvironmentError::WriteLockfile)?;

        temp_lockfile
            .persist(&environment_lockfile_path)
            .map_err(|persist_error| CoreEnvironmentError::WriteLockfile(persist_error.error))?;

        Ok(LockResult::Changed(lockfile))
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
    /// ```ignore
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
    #[must_use = "don't discard the store paths of built environments"]
    pub fn build(&mut self, flox: &Flox) -> Result<BuildEnvOutputs, CoreEnvironmentError> {
        let lockfile_path = CanonicalPath::new(self.lockfile_path())
            .map_err(CoreEnvironmentError::BadLockfilePath)?;
        let lockfile = Lockfile::read_from_file(&lockfile_path)?;

        let service_config_path = maybe_make_service_config_file(flox, &lockfile)?;

        let outputs =
            BuildEnvNix.build(&flox.catalog_client, &lockfile_path, service_config_path)?;
        debug!(?outputs, "built environment");
        Ok(outputs)
    }
}

impl CoreEnvironment<()> {
    /// Create a new out-link for the environment at the given path with a
    /// store-path obtained from [Self::build].
    pub fn link(
        out_link_path: impl AsRef<Path>,
        store_path: &BuiltStorePath,
    ) -> Result<(), CoreEnvironmentError> {
        BuildEnvNix.link(out_link_path, store_path)?;

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
    pub fn new(env_dir: impl AsRef<Path>, include_fetcher: IncludeFetcher) -> Self {
        CoreEnvironment {
            env_dir: env_dir.as_ref().to_path_buf(),
            include_fetcher,
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
    ) -> Result<InstallationAttempt, EnvironmentError> {
        let current_manifest_contents = self.manifest_contents()?;
        let mut installation = insert_packages(&current_manifest_contents, packages)
            .map(|insertion| InstallationAttempt {
                new_manifest: insertion.new_toml.map(|toml| toml.to_string()),
                already_installed: insertion.already_installed,
                built_environments: None,
            })
            .map_err(CoreEnvironmentError::ModifyToml)?;
        if let Some(ref new_manifest) = installation.new_manifest {
            let (store_path, _) = self.transact_with_manifest_contents(new_manifest, flox)?;
            installation.built_environments = Some(store_path);
        }
        Ok(installation)
    }

    /// Uninstall packages from the environment atomically
    ///
    /// Locks the environment first in order to detect and resolve any composition.
    ///
    /// Returns the modified environment if there were no errors.
    pub fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<UninstallationAttempt, EnvironmentError> {
        let current_manifest_contents = self.manifest_contents()?;

        let lockfile: Lockfile = self.lock(flox)?.into();
        let packages_in_includes = if let Some(compose) = lockfile.compose {
            Self::get_includes_for_packages(&packages, &compose.include)?
        } else {
            HashMap::new()
        };

        let install_ids = match Self::get_install_ids(&self.manifest()?, packages) {
            Ok(ids) => ids,
            Err(err) => {
                if let CoreEnvironmentError::PackageNotFound(ref package) = err {
                    if let Some(include) = packages_in_includes.get(package) {
                        return Err(EnvironmentError::Core(
                            CoreEnvironmentError::PackageOnlyIncluded(
                                package.to_string(),
                                include.name.clone(),
                            ),
                        ));
                    }
                };
                return Err(EnvironmentError::Core(err));
            },
        };

        let toml = remove_packages(&current_manifest_contents, &install_ids)
            .map_err(CoreEnvironmentError::ModifyToml)?;
        let (store_path, _) = self.transact_with_manifest_contents(toml.to_string(), flox)?;

        Ok(UninstallationAttempt {
            new_manifest: Some(toml.to_string()),
            still_included: packages_in_includes,
            built_environment_store_paths: Some(store_path),
        })
    }

    /// Get the highest priority included environment which provides each package.
    /// Packages that are not provided by any included environments will be absent from the map.
    fn get_includes_for_packages(
        packages: &[String],
        includes: &[LockedInclude],
    ) -> Result<HashMap<String, LockedInclude>, EnvironmentError> {
        let mut result = HashMap::new();
        for package in packages {
            if let Some(include) = Self::get_include_for_package(package, includes)? {
                result.insert(package.clone(), include);
            }
        }

        Ok(result)
    }

    /// Detect which included environment, if any, provides a given package.
    fn get_include_for_package(
        package: &String,
        includes: &[LockedInclude],
    ) -> Result<Option<LockedInclude>, EnvironmentError> {
        // Reverse of merge order so that we return the highest priority match.
        for include in includes.iter().rev() {
            match Self::get_install_ids(&include.manifest, vec![package.to_string()]) {
                Ok(_) => return Ok(Some(include.clone())),
                Err(CoreEnvironmentError::PackageNotFound(_)) => continue,
                Err(CoreEnvironmentError::MultiplePackagesMatch(_, _)) => continue,
                Err(err) => return Err(EnvironmentError::Core(err)),
            }
        }

        Ok(None)
    }

    /// Resolve "loose" package references (e.g. pkg-paths),
    /// to `install_ids` if unambiguous
    /// so that installation references remain valid for other package operations.
    fn get_install_ids(
        manifest: &Manifest,
        packages: Vec<String>,
    ) -> Result<Vec<String>, CoreEnvironmentError> {
        let mut install_ids = Vec::new();
        for pkg in packages {
            // User passed an install id directly
            if manifest.install.inner().contains_key(&pkg) {
                install_ids.push(pkg);
                continue;
            }

            // User passed a package path to uninstall
            // To support version constraints, we match the provided value against
            // `<pkg-path>` and `<pkg-path>@<version>`.
            let matching_iids_by_pkg_path = manifest
                .install
                .inner()
                .iter()
                .filter(|(_iid, descriptor)| {
                    // Find matching pkg-paths and select for uninstall

                    // If the descriptor is not a catalog descriptor, skip.
                    // flakes descriptors are only matched by install_id.
                    let ManifestPackageDescriptor::Catalog(des) = descriptor else {
                        return false;
                    };

                    // Select if the descriptor's pkg_path matches the user's input
                    if des.pkg_path == pkg {
                        return true;
                    }

                    // Select if the descriptor matches the user's input when the version is included
                    // Future: if we want to allow uninstalling a specific outputs as well,
                    //         parsing of uninstall specs will need to be more sophisticated.
                    //         For now going with a simple check for pkg-path@version.
                    if let Some(version) = &des.version {
                        format!("{}@{}", des.pkg_path, version) == pkg
                    } else {
                        false
                    }
                })
                .map(|(iid, _)| iid.to_owned())
                .collect::<Vec<String>>();

            // Extend the install_ids with the matching install id from pkg-path
            match matching_iids_by_pkg_path.len() {
                0 => return Err(CoreEnvironmentError::PackageNotFound(pkg)),
                // if there is only one package with the given pkg-path, uninstall it
                1 => install_ids.extend(matching_iids_by_pkg_path),
                // if there are multiple packages with the given pkg-path, ask for a specific install id
                _ => {
                    return Err(CoreEnvironmentError::MultiplePackagesMatch(
                        pkg,
                        matching_iids_by_pkg_path,
                    ));
                },
            }
        }
        Ok(install_ids)
    }

    /// Atomically edit this environment, ensuring that it still builds
    pub fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError> {
        let old_contents = self.manifest_contents()?;

        // skip the edit if the contents are unchanged
        // note: consumers of this function may call [Self::link] separately,
        //       causing an evaluation/build of the environment.
        if contents == old_contents {
            return Ok(EditResult::Unchanged);
        }

        let old_lockfile = self.existing_lockfile()?;
        let (store_path, new_lockfile) = self.transact_with_manifest_contents(&contents, flox)?;

        Ok(EditResult::Changed {
            old_lockfile,
            new_lockfile,
            built_environment_store_paths: store_path,
        })
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
    ) -> Result<Result<EditResult, EnvironmentError>, CoreEnvironmentError> {
        let old_contents = self.manifest_contents()?;

        // skip the edit if the contents are unchanged
        // note: consumers of this function may call [Self::link] separately,
        //       causing an evaluation/build of the environment.
        if contents == old_contents {
            return Ok(Ok(EditResult::Unchanged));
        }

        let old_lockfile = self.existing_lockfile()?;

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

        let new_lockfile = match temp_env.lock(flox) {
            Ok(lockfile) => lockfile.into(),
            Err(lock_err) => {
                debug!("transaction: lock failed: {:?}", lock_err);
                debug!("transaction: replacing environment");
                self.replace_with(temp_env)?;
                return Ok(Err(lock_err));
            },
        };

        let build_attempt = temp_env.build(flox);

        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;

        match build_attempt {
            Ok(store_path) => Ok(Ok(EditResult::Changed {
                old_lockfile,
                new_lockfile,
                built_environment_store_paths: store_path,
            })),
            Err(err) => Ok(Err(EnvironmentError::Core(err))),
        }
    }

    /// Atomically upgrade packages in this environment
    ///
    /// First resolve a new lockfile with upgraded packages using the catalog client.
    /// Then verify the new lockfile by building the environment.
    ///
    /// Finally if `write_lockfile` is true,
    /// replace the existing environment with the new, upgraded one.
    /// Otherwise, validate the upgrade by writing the new lockfile to a temporary file
    /// and building it.
    pub fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
        write_lockfile: bool,
    ) -> Result<UpgradeResult, EnvironmentError> {
        tracing::debug!(to_upgrade = groups_or_iids.join(","), "upgrading");
        let manifest = self.manifest()?;

        Self::ensure_valid_upgrade(groups_or_iids, &manifest)?;
        tracing::debug!("using catalog client to upgrade");

        let mut result = self.upgrade_with_catalog_client(flox, groups_or_iids, &manifest)?;

        // SAFETY: serde_json::to_string_pretty is only documented to fail if
        // the "Serialize decides to fail, or if T contains a map with non-string keys",
        // neither of which should happen here.
        let lockfile_contents = serde_json::to_string_pretty(&result.new_lockfile).unwrap();

        if write_lockfile {
            if result.diff().is_empty() {
                return Ok(result);
            }

            let store_path = self.transact_with_lockfile_contents(lockfile_contents, flox)?;
            result.store_path = Some(store_path);
        } else {
            let tmp_lockfile = tempfile::NamedTempFile::new_in(&flox.temp_dir)
                .map_err(CoreEnvironmentError::WriteLockfile)?;
            fs::write(&tmp_lockfile, lockfile_contents)
                .map_err(CoreEnvironmentError::WriteLockfile)?;

            // We are not interested in the store path here, so we ignore the result
            // Neither do we depend on services, so we pass `None`
            let _ = BuildEnvNix
                .build(&flox.catalog_client, tmp_lockfile.path(), None)
                .map_err(|e| EnvironmentError::Core(CoreEnvironmentError::BuildEnv(e)))?;
        }

        Ok(result)
    }

    fn ensure_valid_upgrade(
        groups_or_iids: &[&str],
        manifest: &Manifest,
    ) -> Result<(), CoreEnvironmentError> {
        for id in groups_or_iids {
            tracing::debug!(id, "checking that id is a package or group");
            if *id == "toplevel" {
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
                        pkg: id.to_string(),
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
                        pkg: id.to_string(),
                        group,
                    },
                ));
            }
        }
        Ok(())
    }

    /// Upgrade the given groups or install ids in the environment using the catalog client.
    /// The environment is upgraded by locking the existing manifest
    /// using [LockedManifestCatalog::lock_manifest] with the existing lockfile as a seed,
    /// where the upgraded packages have been filtered out causing them to be re-resolved.
    fn upgrade_with_catalog_client(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
        manifest: &Manifest,
    ) -> Result<UpgradeResult, EnvironmentError> {
        tracing::debug!(to_upgrade = groups_or_iids.join(","), "upgrading");
        let existing_lockfile = 'lockfile: {
            let Ok(lockfile_path) = CanonicalPath::new(self.lockfile_path()) else {
                break 'lockfile None;
            };
            Some(Lockfile::read_from_file(&lockfile_path)?)
        };

        // Create a seed lockfile by "unlocking" (i.e. removing the locked entries of)
        // all packages matching the given groups or iids.
        // If no groups or iids are provided, all packages are unlocked.
        let seed_lockfile = existing_lockfile.clone().map(|mut lockfile| {
            lockfile.unlock_packages_by_group_or_iid(groups_or_iids);
            lockfile
        });

        let upgraded_lockfile = Lockfile::lock_manifest(
            flox,
            manifest,
            seed_lockfile.as_ref(),
            &self.include_fetcher,
        )
        .block_on()?;

        let result = UpgradeResult {
            old_lockfile: existing_lockfile,
            new_lockfile: upgraded_lockfile,
            store_path: None,
        };

        Ok(result)
    }

    /// Upgrade environment with latest changes to included environments.
    ///
    /// This just delegates to Lockfile::lock_manifest_with_include_upgrades and
    /// runs locking boilerplate.
    /// The approach here is not symmetric to the implementation of upgrade().
    /// upgrade() modifies the seed lockfile and then locks normally.
    /// We can't take that approach here because the name of an included
    /// environment may not exist until after it has been fetched.
    /// So we can't verify if a requested upgrade can be performed until
    /// after we've fetched all included environments.
    // TODO: this mostly duplicates logic in lock() and upgrade()
    // We could probably factor some of it out.
    pub fn include_upgrade(
        &mut self,
        flox: &Flox,
        to_upgrade: Vec<String>,
    ) -> Result<UpgradeResult, EnvironmentError> {
        tracing::debug!(
            includes = to_upgrade.iter().join(","),
            "upgrading included environments"
        );

        let manifest = self.manifest()?;

        let existing_lockfile_contents = self.existing_lockfile_contents()?;
        let existing_lockfile = existing_lockfile_contents
            .as_deref()
            .map(Lockfile::from_str)
            .transpose()?;

        let new_lockfile = Lockfile::lock_manifest_with_include_upgrades(
            flox,
            &manifest,
            existing_lockfile.as_ref(),
            &self.include_fetcher,
            Some(to_upgrade),
        )
        .block_on()?;

        let mut result = UpgradeResult {
            old_lockfile: existing_lockfile,
            new_lockfile,
            store_path: None,
        };

        // SAFETY: serde_json::to_string_pretty is only documented to fail if
        // the "Serialize decides to fail, or if T contains a map with non-string keys",
        // neither of which should happen here.
        let lockfile_contents = serde_json::to_string_pretty(&result.new_lockfile).unwrap();

        let environment_lockfile_path = self.lockfile_path();

        if Some(&lockfile_contents) == existing_lockfile_contents.as_ref() {
            debug!(
                ?environment_lockfile_path,
                "lockfile is up to date, skipping write"
            );
            return Ok(result);
        }

        let store_path = self.transact_with_lockfile_contents(lockfile_contents, flox)?;
        result.store_path = Some(store_path);

        Ok(result)
    }

    /// Makes a temporary copy of the environment so modifications to the manifest
    /// can be applied without modifying the original environment.
    fn writable(
        &mut self,
        tempdir: impl AsRef<Path>,
    ) -> Result<CoreEnvironment<ReadWrite>, CoreEnvironmentError> {
        copy_dir_recursive(&self.env_dir, tempdir.as_ref(), true)
            .map_err(CoreEnvironmentError::MakeTemporaryEnv)?;

        Ok(CoreEnvironment {
            env_dir: tempdir.as_ref().to_path_buf(),
            include_fetcher: self.include_fetcher.clone(),
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
    ) -> Result<(BuildEnvOutputs, Lockfile), EnvironmentError> {
        let manifest: Manifest = toml::from_str(manifest_contents.as_ref())
            .map_err(CoreEnvironmentError::DeserializeManifest)?;
        manifest
            .services
            .validate()
            .map_err(|e| EnvironmentError::Core(CoreEnvironmentError::Services(e)))?;

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
        let lockfile = temp_env.lock(flox)?.into();

        debug!("transaction: building environment");
        let store_path = temp_env.build(flox)?;

        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;
        Ok((store_path, lockfile))
    }

    /// Attempt to transactionally replace the lockfile contents
    ///
    /// TODO: this is separate from transact_with_manifest_contents because it
    /// shouldn't have to call lock. Currently build calls lock, but we
    /// shouldn't have to lock a second time.
    #[must_use = "don't discard the store path of built environments"]
    fn transact_with_lockfile_contents(
        &mut self,
        lockfile_contents: impl AsRef<str>,
        flox: &Flox,
    ) -> Result<BuildEnvOutputs, CoreEnvironmentError> {
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

#[derive(Debug, Clone, PartialEq)]
pub enum EditResult {
    /// The manifest was not modified.
    Unchanged,
    /// The manifest was modified, although the change could be as minimal as
    /// whitespace
    Changed {
        old_lockfile: Option<Lockfile>,
        new_lockfile: Lockfile,
        built_environment_store_paths: BuildEnvOutputs,
    },
}

impl EditResult {
    /// The user needs to re-activate to have changes made to the environment
    /// take effect
    pub fn reactivate_required(&self) -> bool {
        match self {
            Self::Unchanged => false,
            Self::Changed {
                old_lockfile,
                new_lockfile,
                ..
            } => {
                let hook_changed = old_lockfile
                    .as_ref()
                    .and_then(|lockfile| lockfile.manifest.hook.as_ref())
                    != new_lockfile.manifest.hook.as_ref();

                let vars_changed = old_lockfile
                    .as_ref()
                    .map(|lockfile| lockfile.manifest.vars.clone())
                    .unwrap_or_default()
                    != new_lockfile.manifest.vars;

                let profile_changed = old_lockfile
                    .as_ref()
                    .and_then(|lockfile| lockfile.manifest.profile.as_ref())
                    != new_lockfile.manifest.profile.as_ref();

                hook_changed || vars_changed || profile_changed
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct UpgradeResult {
    pub old_lockfile: Option<Lockfile>,
    pub new_lockfile: Lockfile,
    pub store_path: Option<BuildEnvOutputs>,
}

/// Packages that have upgrades in the format
/// packages[install_id][system] = (old_package, new_package)
pub type UpgradeDiff = BTreeMap<String, BTreeMap<String, (LockedPackage, LockedPackage)>>;

/// Packages for a single system that have upgrades in the format
/// packages[install_id] = (old_package, new_package)
pub type SingleSystemUpgradeDiff = BTreeMap<String, (LockedPackage, LockedPackage)>;

impl UpgradeResult {
    /// Return an iterator over sorted install IDs that have an upgrade
    pub fn packages(&self) -> impl Iterator<Item = String> {
        self.diff().into_keys().sorted()
    }

    /// Return a map of packages that have upgrades in the format
    /// packages[install_id][system] = (old_package, new_package)
    pub fn diff(&self) -> UpgradeDiff {
        // Record a nested map where you retrieve the locked package
        // via pkgs[install_id][system]
        let previous_packages = if let Some(ref lockfile) = self.old_lockfile {
            let mut pkgs_by_id = BTreeMap::new();
            lockfile.packages.iter().for_each(|pkg| {
                let by_system = pkgs_by_id
                    .entry(pkg.install_id().to_owned())
                    .or_insert(BTreeMap::new());
                by_system.entry(pkg.system().clone()).or_insert(pkg.clone());
            });
            pkgs_by_id
        } else {
            BTreeMap::new()
        };

        let mut pkgs_after_upgrade = {
            let mut pkgs_by_id = BTreeMap::new();
            self.new_lockfile.packages.iter().for_each(|pkg| {
                let by_system = pkgs_by_id
                    .entry(pkg.install_id().to_owned())
                    .or_insert(BTreeMap::new());
                by_system.entry(pkg.system().clone()).or_insert(pkg.clone());
            });
            pkgs_by_id
        };

        let mut packages_with_upgrades: UpgradeDiff = BTreeMap::new();

        // In some cases you may encounter a package that was in the old lockfile
        // and isn't in the new lockfile (or isn't present for a certain system).
        // We've encountered this in production, which most likely means that the
        // manifest that was present when initiating the upgrade check was out of
        // sync with its lockfile (i.e. someone edited the manifest through means
        // other than `flox edit`). In those cases we silently ignore the packages
        // (or packages for a certain system) that are no longer present.
        for (prev_install_id, prev_packages_by_system) in previous_packages.into_iter() {
            if let Some(mut after_packages_by_system) = pkgs_after_upgrade.remove(&prev_install_id)
            {
                for (prev_system, prev_package) in prev_packages_by_system {
                    if let Some(after_package) = after_packages_by_system.remove(&prev_system) {
                        // Store paths return None for the derivation,
                        // and we shouldn't say store paths have an upgrade.
                        if prev_package.derivation().is_some()
                            && after_package.derivation().is_some()
                            && prev_package.derivation() != after_package.derivation()
                        {
                            let by_system = packages_with_upgrades
                                .entry(prev_install_id.to_owned())
                                .or_default();
                            by_system.insert(prev_system.to_owned(), (prev_package, after_package));
                        }
                    }
                }
            }
        }

        packages_with_upgrades
    }

    /// Return a map of packages that have upgrades in the format
    /// packages[install_id] = (old_package, new_package)
    pub fn diff_for_system(&self, system: &str) -> SingleSystemUpgradeDiff {
        self.diff()
            .into_iter()
            .filter_map(|(install_id, mut by_system)| {
                by_system
                    .remove(system)
                    .map(|(old_package, new_package)| (install_id, (old_package, new_package)))
            })
            .collect()
    }

    /// Returns the names of includes that were changed
    ///
    /// If an include exists in new_lockfile but not old_lockfile, that is
    /// treated as changed
    pub fn include_diff(&self) -> Vec<String> {
        let old_include = self
            .old_lockfile
            .as_ref()
            .and_then(|old_lockfile| old_lockfile.compose.as_ref())
            .map(|old_compose| &old_compose.include);

        let Some(new_compose) = &self.new_lockfile.compose else {
            return vec![];
        };
        let new_include = &new_compose.include;

        // If there aren't any old locked includes, all includes have been
        // changed
        let Some(old_include) = old_include else {
            return new_include
                .iter()
                .map(|locked_include| locked_include.name.clone())
                .collect();
        };

        new_include
            .iter()
            .filter(|new_locked_include| !old_include.contains(new_locked_include))
            .map(|locked_include| locked_include.name.clone())
            .collect()
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
    /// Tried to uninstall a package that wasn't installed
    #[error("couldn't uninstall '{0}', wasn't previously installed")]
    PackageNotFound(String),
    /// Tried to uninstall a package that was only provided by an include.
    #[error(
        "Cannot remove included package '{0}'\n\
         Remove the package from environment '{1}' and then run 'flox include upgrade'"
    )]
    PackageOnlyIncluded(String, String),
    // Multiple packages match user input, must specify install_id
    #[error(
        "multiple packages match '{0}', please specify an install id from possible matches: {1:?}"
    )]
    MultiplePackagesMatch(String, Vec<String>),
    // endregion
    #[error(transparent)]
    Resolve(ResolveError),

    #[error(transparent)]
    BadLockfilePath(CanonicalizeError),

    // todo: refactor upgrade to use `LockedManifest`
    #[error("failed to upgrade environment")]
    UpgradeFailedCatalog(#[source] UpgradeError),
    // endregion
    #[error("could not automatically migrate manifest to version 1")]
    MigrateManifest(#[source] toml_edit::de::Error),

    #[error("failed to create version 1 lock")]
    LockForMigration(#[source] Box<CoreEnvironmentError>),

    #[error(transparent)]
    Services(#[from] ServiceError),

    #[error(transparent)]
    BuildEnv(#[from] BuildEnvError),

    // region: lockfile errors
    #[error("could not open lockfile")]
    ReadLockfile(#[source] std::io::Error),

    /// when parsing the contents of a lockfile into a [LockedManifest]
    #[error("could not parse lockfile")]
    ParseLockfile(#[source] serde_json::Error),
    // endregion
}

impl CoreEnvironmentError {
    pub fn is_incompatible_system_error(&self) -> bool {
        // incomaptible system errors during resolution
        let is_lock_incompatible_system_error = matches!(
            self,
            CoreEnvironmentError::Resolve(ResolveError::ResolutionFailed(failures))
             if failures.0.iter().any(|f| matches!(f, ResolutionFailure::PackageUnavailableOnSomeSystems { .. })));

        // Incomaptible system errors during build
        // i.e. trying to build a lockfile that specifies systems,
        // but the current system is not in the list
        let is_build_incompatible_system_error = matches!(
            self,
            CoreEnvironmentError::BuildEnv(BuildEnvError::LockfileIncompatible { .. })
        );

        is_lock_incompatible_system_error || is_build_incompatible_system_error
    }
}

pub mod test_helpers {
    use indoc::indoc;

    use super::*;
    use crate::flox::Flox;
    use crate::models::environment::fetcher::test_helpers::mock_include_fetcher;

    #[cfg(target_os = "macos")]
    pub const MANIFEST_INCOMPATIBLE_SYSTEM: &str = indoc! {r#"
        version = 1
        [options]
        systems = ["x86_64-linux"]
        "#};
    #[cfg(target_os = "macos")]
    pub const MANIFEST_INCOMPATIBLE_SYSTEM_V1: &str = indoc! {r#"
        version = 1
        [options]
        systems = ["x86_64-linux"]
        "#};
    #[cfg(target_os = "linux")]
    pub const MANIFEST_INCOMPATIBLE_SYSTEM: &str = indoc! {r#"
        version = 1
        [options]
        systems = ["aarch64-darwin"]
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

        CoreEnvironment::new(&env_path, mock_include_fetcher())
    }

    pub fn new_core_environment_with_lockfile(
        flox: &Flox,
        manifest_contents: &str,
        lockfile_contents: &str,
    ) -> CoreEnvironment {
        let env_path = tempfile::tempdir_in(&flox.temp_dir).unwrap().into_path();
        fs::write(env_path.join(MANIFEST_FILENAME), manifest_contents).unwrap();
        fs::write(env_path.join(LOCKFILE_FILENAME), lockfile_contents).unwrap();

        CoreEnvironment::new(&env_path, mock_include_fetcher())
    }

    pub fn new_core_environment_from_env_files(
        flox: &Flox,
        env_files_dir: impl AsRef<Path>,
    ) -> CoreEnvironment {
        let env_files_dir = env_files_dir.as_ref();
        let manifest_contents = fs::read_to_string(env_files_dir.join(MANIFEST_FILENAME)).unwrap();
        let lockfile_contents = fs::read_to_string(env_files_dir.join(LOCKFILE_FILENAME)).unwrap();
        new_core_environment_with_lockfile(flox, &manifest_contents, &lockfile_contents)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::os::unix::fs::PermissionsExt;

    use catalog::test_helpers::reset_mocks_from_file;
    use catalog::{GENERATED_DATA, MANUALLY_GENERATED};
    use catalog_api_v1::types::{ResolvedPackageDescriptor, SystemEnum};
    use chrono::{DateTime, Utc};
    use flox_core::Version;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use tempfile::{TempDir, tempdir_in};
    use test_helpers::{new_core_environment_from_env_files, new_core_environment_with_lockfile};
    use tests::test_helpers::MANIFEST_INCOMPATIBLE_SYSTEM;

    use self::catalog::{CatalogPage, MockClient, ResolvedPackageGroup};
    use self::test_helpers::new_core_environment;
    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::lockfile;
    use crate::models::lockfile::test_helpers::fake_catalog_package_lock;
    use crate::models::manifest::typed::{DEFAULT_GROUP_NAME, PackageDescriptorCatalog};
    use crate::providers::catalog::{self, Client};
    use crate::providers::services::SERVICE_CONFIG_FILENAME;

    /// Create a CoreEnvironment with an empty manifest (with version = 1)
    fn empty_core_environment() -> (CoreEnvironment, Flox, TempDir) {
        let (flox, tempdir) = flox_instance();

        (new_core_environment(&flox, "version = 1"), flox, tempdir)
    }

    /// Check that `edit` updates the manifest and creates a lockfile
    #[test]
    #[cfg(feature = "impure-unit-tests")]
    fn edit_env_creates_manifest_and_lockfile() {
        let (mut flox, tempdir) = flox_instance();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        fs::write(env_path.path().join(MANIFEST_FILENAME), "version = 1").unwrap();

        let mut env_view = CoreEnvironment::new(&env_path, IncludeFetcher {
            base_directory: None,
        });

        let new_env_str = r#"
        version = 1

        [install]
        hello.pkg-path = "hello"
        "#;

        reset_mocks_from_file(&mut flox.catalog_client, "resolve/hello.json");
        env_view.edit(&flox, new_env_str.to_string()).unwrap();

        assert_eq!(env_view.manifest_contents().unwrap(), new_env_str);
        assert!(env_view.env_dir.join(LOCKFILE_FILENAME).exists());
    }

    /// A no-op with edit returns EditResult::Unchanged
    #[test]
    fn edit_no_op_returns_unchanged() {
        let (flox, _temp_dir_handle) = flox_instance();
        let mut env_view = new_core_environment(&flox, "version = 1");

        let result = env_view.edit(&flox, "version = 1".to_string()).unwrap();

        assert!(matches!(result, EditResult::Unchanged));
    }

    /// Trying to build a manifest with a system other than the current one
    /// results in an error that is_incompatible_system_error()
    #[test]
    fn build_incompatible_system() {
        let (flox, _temp_dir_handle) = flox_instance();
        let mut env_view = new_core_environment(&flox, "");
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

    #[test]
    fn built_environments_generate_service_config() {
        let (flox, _dir) = flox_instance();

        // Manifest with a services section
        let contents = indoc! {r#"
        version = 1

        [services.foo]
        command = "start foo"
        "#};

        let mut env = new_core_environment(&flox, contents);
        env.lock(&flox).unwrap();

        // Build the environment and verify that the config file exists
        let store_path = env.build(&flox).unwrap();
        let config_path = store_path.develop.join(SERVICE_CONFIG_FILENAME);
        assert!(config_path.exists());
    }

    /// Installing hello with edit returns EditResult::Changed and
    /// reactivate_required() returns false
    #[test]
    fn edit_adding_package_returns_changed() {
        let (mut env_view, mut flox, _temp_dir_handle) = empty_core_environment();

        let new_env_str = r#"
        version = 1

        [install]
        hello.pkg-path = "hello"
        "#;

        reset_mocks_from_file(&mut flox.catalog_client, "resolve/hello.json");
        let result = env_view.edit(&flox, new_env_str.to_string()).unwrap();

        assert!(matches!(result, EditResult::Changed { .. }));
        assert!(!result.reactivate_required());
    }

    /// After adding a hook with edit, reactivate_required returns true
    #[test]
    fn edit_adding_hook_returns_reactivate_required() {
        let (mut env_view, flox, _temp_dir_handle) = empty_core_environment();

        let new_env_str = r#"
        version = 1

        [hook]
        on-activate = ""
        "#;

        let result = env_view.edit(&flox, new_env_str.to_string()).unwrap();

        assert!(result.reactivate_required());
    }

    /// Check that with an empty list of packages to upgrade, all packages are upgraded
    // TODO: add fixtures for resolve mocks if we add more of these tests
    #[test]
    fn upgrade_with_empty_list_upgrades_all() {
        let (mut env_view, mut flox, _temp_dir_handle) = empty_core_environment();

        let mut manifest = Manifest::default();
        let (foo_iid, foo_descriptor, foo_locked) = fake_catalog_package_lock("foo", None);
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor);
        let lockfile = lockfile::Lockfile {
            version: Version,
            packages: vec![foo_locked.into()],
            manifest: manifest.clone(),
            compose: None,
        };

        let lockfile_str = serde_json::to_string_pretty(&lockfile).unwrap();

        fs::write(env_view.lockfile_path(), lockfile_str).unwrap();

        let mut mock_client = MockClient::new(None::<&str>).unwrap();
        mock_client.push_resolve_response(vec![ResolvedPackageGroup {
            name: DEFAULT_GROUP_NAME.to_string(),
            page: Some(CatalogPage {
                packages: Some(vec![ResolvedPackageDescriptor {
                    catalog: None,
                    attr_path: "foo".to_string(),
                    pkg_path: "foo".to_string(),
                    broken: Some(false),
                    derivation: "new derivation".to_string(),
                    description: Some("description".to_string()),
                    insecure: Some(false),
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
                    scrape_date: Some(DateTime::<Utc>::MIN_UTC),
                    stabilities: None,
                    unfree: None,
                    version: "1.0".to_string(),
                    system: SystemEnum::Aarch64Darwin,
                    cache_uri: None,
                    missing_builds: None,
                }]),
                msgs: vec![],
                page: 1,
                url: "url".to_string(),
                complete: true,
            }),
            msgs: vec![],
        }]);
        flox.catalog_client = Client::Mock(mock_client);

        let upgraded_packages = env_view
            .upgrade_with_catalog_client(&flox, &[], &manifest)
            .unwrap()
            .diff();

        assert!(upgraded_packages.len() == 1);
    }

    /// replacing an environment should fail if a backup exists
    #[test]
    fn detects_existing_backup() {
        let (_flox, tempdir) = flox_instance();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        let sandbox_path = tempfile::tempdir_in(&tempdir).unwrap();
        fs::create_dir(env_path.path().with_extension("tmp")).unwrap();

        let mut env_view = CoreEnvironment::new(&env_path, IncludeFetcher {
            base_directory: None,
        });
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

        let mut env_view = CoreEnvironment::new(&env_path, IncludeFetcher {
            base_directory: None,
        });
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
    #[cfg(feature = "impure-unit-tests")]
    fn build_flox_environment_and_links() {
        let (mut flox, tempdir) = flox_instance();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        fs::write(
            env_path.path().join(MANIFEST_FILENAME),
            r#"
        version = 1

        [install]
        hello.pkg-path = "hello"
        "#,
        )
        .unwrap();

        let mut env_view = CoreEnvironment::new(&env_path, IncludeFetcher {
            base_directory: None,
        });

        reset_mocks_from_file(&mut flox.catalog_client, "resolve/hello.json");
        env_view.lock(&flox).expect("locking should succeed");
        let store_path = env_view.build(&flox).expect("build should succeed");
        CoreEnvironment::link(
            env_path.path().with_extension("out-link"),
            &store_path.develop,
        )
        .expect("link should succeed");

        // very rudimentary check that the environment manifest built correctly
        // and linked to the out-link.
        assert!(
            env_path
                .path()
                .with_extension("out-link")
                .join("bin/hello")
                .exists()
        );
    }

    #[test]
    fn v1_does_not_need_relock() {
        let (flox, _temp_dir_handle) = flox_instance();
        let environment =
            new_core_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        assert!(environment.lockfile_if_up_to_date().unwrap().is_some());
    }

    #[test]
    fn modified_v1_needs_relock() {
        let (flox, _temp_dir_handle) = flox_instance();
        let manifest_contents =
            fs::read_to_string(MANUALLY_GENERATED.join("empty").join(MANIFEST_FILENAME)).unwrap();
        let lockfile_contents =
            fs::read_to_string(GENERATED_DATA.join("envs/hello").join(LOCKFILE_FILENAME)).unwrap();
        let environment =
            new_core_environment_with_lockfile(&flox, &manifest_contents, &lockfile_contents);
        assert!(environment.lockfile_if_up_to_date().unwrap().is_none());
    }

    #[test]
    fn lock_does_not_write_lockfile_if_unchanged() {
        let (flox, _temp_dir_handle) = flox_instance();
        let mut environment =
            new_core_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        let mtime_original = environment
            .lockfile_path()
            .metadata()
            .unwrap()
            .modified()
            .unwrap();

        let _ = environment.lock(&flox).unwrap();

        let mtime_after = environment
            .lockfile_path()
            .metadata()
            .unwrap()
            .modified()
            .unwrap();

        assert_eq!(mtime_after, mtime_original);
    }

    /// Locking an environment should write a lockfile
    /// if the contents change compared to the existing lockfile,
    /// even if the contents are semantically equivalent.
    #[test]
    fn lock_writes_if_contents_change() {
        let (flox, _temp_dir_handle) = flox_instance();
        let mut environment =
            new_core_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        // add some whitespace to the file, that simulates different original content
        // we subsequently expect the file to be modified
        {
            let mut lockfile = OpenOptions::new()
                .read(true)
                .append(true)
                .open(environment.lockfile_path())
                .unwrap();

            writeln!(lockfile, "\n\n\n",).unwrap();

            // fsync metadata to ensure the mtime is updated
            lockfile.sync_all().unwrap();
        }

        let mtime_original = environment
            .lockfile_path()
            .metadata()
            .unwrap()
            .modified()
            .unwrap();

        let _ = environment.lock(&flox).unwrap();

        let mtime_after = environment
            .lockfile_path()
            .metadata()
            .unwrap()
            .modified()
            .unwrap();

        assert_ne!(mtime_after, mtime_original);
    }

    /// UNINSTALL TESTS
    ///
    /// Generates a mock `TypedManifest` for testing purposes.
    /// This function is designed to simplify the creation of test data by
    /// generating a `TypedManifest` based on a list of install IDs and
    /// package paths.
    /// # Arguments
    ///
    /// * `entries` - A vector of tuples, where each tuple contains an install
    ///   ID and a package path.
    ///
    /// # Returns
    ///
    /// * `TypedManifest` - A mock `TypedManifest` containing the provided entries.
    fn generate_mock_manifest(entries: Vec<(&str, &str)>) -> Manifest {
        let mut typed_manifest_mock = Manifest::default();

        for (test_iid, dotted_package) in entries {
            typed_manifest_mock.install.inner_mut().insert(
                test_iid.to_string(),
                ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog {
                    pkg_path: dotted_package.to_string(),
                    pkg_group: None,
                    priority: None,
                    version: None,
                    systems: None,
                }),
            );
        }

        typed_manifest_mock
    }
    /// Return the install ID if it matches the user input
    #[test]
    fn test_get_install_ids_to_uninstall_by_install_id() {
        let manifest_mock = generate_mock_manifest(vec![("testInstallID", "dotted.package")]);
        let result =
            CoreEnvironment::get_install_ids(&manifest_mock, vec!["testInstallID".to_string()])
                .unwrap();
        assert_eq!(result, vec!["testInstallID".to_string()]);
    }

    #[test]
    /// Return the install ID if a pkg-path matches the user input
    fn test_get_install_ids_to_uninstall_by_pkg_path() {
        let manifest_mock = generate_mock_manifest(vec![("testInstallID", "dotted.package")]);
        let result =
            CoreEnvironment::get_install_ids(&manifest_mock, vec!["dotted.package".to_string()])
                .unwrap();
        assert_eq!(result, vec!["testInstallID".to_string()]);
    }

    #[test]
    /// Ensure that the install ID takes precedence over pkg-path when both are present
    fn test_get_install_ids_to_uninstall_iid_wins() {
        let manifest_mock = generate_mock_manifest(vec![
            ("testInstallID1", "dotted.package"),
            ("testInstallID2", "dotted.package"),
            ("dotted.package", "dotted.package"),
        ]);

        let result =
            CoreEnvironment::get_install_ids(&manifest_mock, vec!["dotted.package".to_string()])
                .unwrap();
        assert_eq!(result, vec!["dotted.package".to_string()]);
    }

    #[test]
    /// Throw an error when multiple packages match by pkg_path and flox can't determine which to uninstall
    fn test_get_install_ids_to_uninstall_multiple_pkg_paths_match() {
        let manifest_mock = generate_mock_manifest(vec![
            ("testInstallID1", "dotted.package"),
            ("testInstallID2", "dotted.package"),
            ("testInstallID3", "dotted.package"),
        ]);
        let result =
            CoreEnvironment::get_install_ids(&manifest_mock, vec!["dotted.package".to_string()])
                .unwrap_err();
        assert!(matches!(
            result,
            CoreEnvironmentError::MultiplePackagesMatch(_, _)
        ));
    }

    #[test]
    /// Throw an error if no install ID or pkg-path matches the user input
    fn test_get_install_ids_to_uninstall_pkg_not_found() {
        let manifest_mock = generate_mock_manifest(vec![("testInstallID1", "dotted.package")]);
        let result = CoreEnvironment::get_install_ids(&manifest_mock, vec![
            "invalid.packageName".to_string(),
        ])
        .unwrap_err();
        assert!(matches!(result, CoreEnvironmentError::PackageNotFound(_)));
    }

    #[test]
    fn test_get_install_ids_to_uninstall_with_version() {
        let mut manifest_mock = generate_mock_manifest(vec![("testInstallID", "dotted.package")]);

        if let ManifestPackageDescriptor::Catalog(descriptor) = manifest_mock
            .install
            .inner_mut()
            .get_mut("testInstallID")
            .unwrap()
        {
            descriptor.version = Some("1.0".to_string());
        };

        let result = CoreEnvironment::get_install_ids(&manifest_mock, vec![
            "dotted.package@1.0".to_string(),
        ])
        .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "testInstallID");
    }

    #[test]
    fn edit_fails_when_daemon_has_no_shutdown_command() {
        let (flox, _dir) = flox_instance();
        let initial_manifest = r#"
            version = 1
        "#;
        let mut env = new_core_environment(&flox, initial_manifest);
        let bad_manifest = r#"
            version = 1

            [services.bad]
            command = "cmd"
            is-daemon = true
            # missing shutdown.command = "..."
        "#;
        let res = env.transact_with_manifest_contents(bad_manifest, &flox);
        eprintln!("{res:?}");
        assert!(matches!(
            res,
            Err(EnvironmentError::Core(CoreEnvironmentError::Services(
                ServiceError::InvalidConfig(_)
            )))
        ));
    }
}
