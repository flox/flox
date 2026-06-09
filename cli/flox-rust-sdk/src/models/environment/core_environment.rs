use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use flox_core::{WriteError, write_atomically};
use flox_manifest::interfaces::{
    AsLatestSchema,
    AsTypedOnlyManifest,
    AsWritableManifest,
    ContentsMatch,
    OriginalSchemaVersion,
    PackageLookup,
    SchemaVersion,
    WriteManifest,
};
use flox_manifest::lockfile::{LOCKFILE_FILENAME, LockedPackage, Lockfile, LockfileError};
use flox_manifest::parsed::common::KnownSchemaVersion;
use flox_manifest::raw::{ModifyPackages, PackageToInstall, TomlEditError};
use flox_manifest::{MANIFEST_FILENAME, Manifest, ManifestError, Migrated, Validated, Writable};
use itertools::Itertools;
use pollster::FutureExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

use super::fetcher::IncludeFetcher;
use super::uninstall::{UninstallSpec, resolve_specs_to_modifications};
use super::{
    CanonicalizeError,
    EnvironmentError,
    InstallationAttempt,
    UninstallationAttempt,
    UpgradeError,
    copy_dir_recursive,
};
use crate::data::CanonicalPath;
use crate::flox::Flox;
use crate::models::environment::install::compute_install_modifications;
use crate::providers::buildenv::{BuildEnv, BuildEnvError, BuildEnvNix, BuildEnvOutputs};
use crate::providers::lock_manifest::{LockManifest, LockResult, ResolutionFailure, ResolveError};
use crate::providers::nix_auth::{AuthError, NixAuth};
use crate::providers::services::process_compose::{ServiceError, maybe_make_service_config_file};

pub struct ReadOnly {}
struct ReadWrite {}

/// Name of the exclusive transaction lock file, kept inside the environment
/// directory while a mutation transaction is in progress.
const TRANSACTION_LOCK_FILENAME: &str = "transaction.lock";

/// Identifies the holder of an in-progress environment transaction.
///
/// Written as JSON to the transaction lock file so that users sharing an
/// environment over NFS can identify which host and process is editing.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct TransactionLockInfo {
    pid: u32,
    hostname: String,
    username: String,
}

/// RAII guard that removes the transaction lock file when dropped.
///
/// Stores the inode of the lock file at acquisition time and only removes the
/// file if its inode still matches — protecting against the race where a user
/// manually deletes a stale lock while the original holder is still running and
/// a second process creates a new lock at the same path.
#[derive(Debug)]
struct TransactionLock {
    lock_path: PathBuf,
    /// Inode of lock_path at acquisition time, used to verify ownership on drop.
    inode: u64,
}

impl Drop for TransactionLock {
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt as _;
        if let Ok(meta) = fs::metadata(&self.lock_path)
            && meta.ino() == self.inode
        {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

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

    pub fn manifest_without_migrating(&self) -> Result<Manifest<Validated>, ManifestError> {
        Manifest::read_typed(self.manifest_path())
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

    /// Return a [LockedManifest] if the environment is already locked and has
    /// the same manifest contents as the manifest, otherwise return None.
    /// Note that the manifest could have whitespace or comment differences from
    /// the lockfile.
    pub fn lockfile_if_up_to_date(&self) -> Result<Option<Lockfile>, CoreEnvironmentError> {
        let lockfile_path = self.lockfile_path();

        let Ok(lockfile_path) = CanonicalPath::new(lockfile_path) else {
            return Ok(None);
        };

        let lockfile = Lockfile::read_from_file(&lockfile_path)?;

        // Check if the manifest embedded in the lockfile and the manifest
        // itself have the same contents
        let serialized_unmigrated_manifest = &self.manifest_without_migrating()?.as_typed_only();
        let already_locked =
            lockfile.is_up_to_date_with_serialized_manifest(serialized_unmigrated_manifest);

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

    fn ensure_manifest_schemas_match(
        &mut self,
        original_schema: KnownSchemaVersion,
        lockfile: &mut Lockfile,
    ) -> Result<(), EnvironmentError> {
        let schema_after_merging_and_locking = lockfile.manifest.get_schema_version();
        let on_disk_manifest_needs_migration = schema_after_merging_and_locking != original_schema;

        if on_disk_manifest_needs_migration {
            let migrated_manifest = self.manifest_without_migrating()?.migrate(Some(lockfile))?;
            migrated_manifest
                .as_writable()
                .write_to_file(self.manifest_path())?;
            if let Some(compose) = lockfile.compose.as_mut() {
                compose.composer = migrated_manifest.as_typed_only().clone();
            }
        }

        Ok(())
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
        let manifest_without_migrating = self.manifest_without_migrating()?.as_typed_only();
        let original_schema = manifest_without_migrating.get_schema_version();

        let existing_lockfile = self.existing_lockfile()?;
        let migrated_manifest_for_locking =
            manifest_without_migrating.migrate_typed_only(existing_lockfile.as_ref())?;

        // If a lockfile exists, it is used as a base.
        //
        // This is `mut` because we may need to update the on-disk manifest to match the
        // schema of the merged manifest, and then we'll also have to update the
        // `compose.composer` to match.
        let mut lockfile = LockManifest::lock_manifest(
            flox,
            &migrated_manifest_for_locking,
            existing_lockfile.as_ref(),
            &self.include_fetcher,
        )
        .block_on()?;

        // Now that we have an up to date lockfile, we check the schema version of its manifest
        // and ensure that the manifest on disk and the manifest in the lockfile are all in sync.
        self.ensure_manifest_schemas_match(original_schema, &mut lockfile)?;

        let mut lockfile_contents =
            serde_json::to_string_pretty(&lockfile).expect("lockfile structure is valid json");
        lockfile_contents.push('\n');

        let environment_lockfile_path = self.lockfile_path();

        if Some(&lockfile) == existing_lockfile.as_ref() {
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

        write_atomically(&environment_lockfile_path, lockfile_contents)
            .map_err(CoreEnvironmentError::WriteLockfileAtomically)?;

        Ok(LockResult::Changed(lockfile))
    }

    /// Lock the provided manifest
    ///
    /// Note: does not handle updating the on-disk manifest or lockfile,
    /// and does not handle ensuring that the schemas of the on-disk
    /// manifest and the lockfile's manifest are in sync.
    pub fn lock_without_writing(
        &mut self,
        flox: &Flox,
        manifest: &Manifest<Migrated>,
        existing_lockfile: Option<&Lockfile>,
    ) -> Result<LockResult, EnvironmentError> {
        // If a lockfile exists, it is used as a base.
        let lockfile = LockManifest::lock_manifest(
            flox,
            &manifest.as_migrated_typed_only(),
            existing_lockfile,
            &self.include_fetcher,
        )
        .block_on()?;

        if Some(&lockfile) == existing_lockfile {
            return Ok(LockResult::Unchanged(lockfile));
        }

        Ok(LockResult::Changed(lockfile))
    }

    /// Build the environment.
    ///
    /// Technically this does write to disk as a side effect for now.
    /// It's included in the [ReadOnly] struct for ergonomic reasons
    /// and because it doesn't modify the manifest.
    ///
    /// Does not lock the manifest.
    /// Call [Self::lock] explicitly before building if the lockfile may be stale.
    ///
    /// Pass `out_link_prefix` to have `nix build` register GC roots and write
    /// the activation symlinks atomically as part of the build.  The prefix
    /// must be `<run_dir>/<system>.<name>`; nix appends `-dev` and `-run` to
    /// produce the two output symlinks.  Pass `None` for validate-only or
    /// pre-flight builds where no symlinks or GC roots are needed.
    #[must_use = "don't discard the store paths of built environments"]
    pub fn build(
        &mut self,
        flox: &Flox,
        out_link_prefix: Option<&Path>,
    ) -> Result<BuildEnvOutputs, CoreEnvironmentError> {
        let lockfile_path = CanonicalPath::new(self.lockfile_path())
            .map_err(CoreEnvironmentError::BadLockfilePath)?;
        let lockfile = Lockfile::read_from_file(&lockfile_path)?;

        let service_config_path = maybe_make_service_config_file(flox, &lockfile)?;

        let auth = NixAuth::from_flox(flox).map_err(CoreEnvironmentError::Auth)?;

        let outputs = BuildEnvNix::new(auth).build(
            &flox.catalog_client,
            &lockfile_path,
            service_config_path,
            out_link_prefix,
        )?;
        debug!(?outputs, "built environment");
        Ok(outputs)
    }
}

/// Core environment mutation methods.
///
/// Since files referenced by the environment are ingested into the nix store,
/// the same [CoreEnvironment] instance can be used even if the concrete
/// [super::Environment] tracks the files in a different way (e.g. a git
/// repository or a database).  Callers may supply `out_link_prefix` to have
/// activation symlinks created as part of the transaction build; passing `None`
/// skips out-link creation.
impl CoreEnvironment<ReadOnly> {
    /// Create a new environment view given the path to a directory that
    /// contains a valid manifest.
    pub fn new(env_dir: impl AsRef<Path>, include_fetcher: IncludeFetcher) -> Self {
        CoreEnvironment {
            env_dir: env_dir.as_ref().to_path_buf(),
            include_fetcher,
            _state: ReadOnly {},
        }
    }

    pub(crate) fn manifest(&mut self, flox: &Flox) -> Result<Manifest<Migrated>, EnvironmentError> {
        let manifest = self.manifest_without_migrating()?;
        let lockfile = self.ensure_locked(flox)?.into();
        let migrated = manifest.migrate(Some(&lockfile))?;
        Ok(migrated)
    }

    /// Install packages to the environment atomically
    ///
    /// Skips rebuilding if all packages are already installed
    pub fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
        out_link_prefix: Option<&Path>,
    ) -> Result<InstallationAttempt, EnvironmentError> {
        let manifest = self.manifest(flox)?;
        // TODO: this could lead to double resolution and surprising errors
        // (e.g. if you try to install a package and we fail to resolve a different package)
        // We need a lockfile for logic about output merging
        let lockfile: Lockfile = self.lock(flox)?.into();

        let modifications = compute_install_modifications(packages, &manifest, &lockfile)?;

        let built_environments = if modifications.is_empty() {
            None
        } else {
            let new_manifest = manifest.modify_packages(&modifications)?;
            let (built_environments, _) =
                self.transact_with_manifest(&new_manifest, flox, out_link_prefix)?;
            Some(built_environments)
        };

        Ok(InstallationAttempt {
            modifications,
            built_environments,
        })
    }

    /// Uninstall packages and or package outputs from the environment atomically
    ///
    /// Locks the environment first in order to detect and resolve any composition.
    ///
    /// Returns the modified environment if there were no errors.
    pub fn uninstall(
        &mut self,
        uninstall_specs: Vec<UninstallSpec>,
        flox: &Flox,
        out_link_prefix: Option<&Path>,
    ) -> Result<UninstallationAttempt, EnvironmentError> {
        // TODO: this could lead to double resolution and surprising errors
        // (e.g. if you try to uninstall a package and we fail to resolve that package)
        // We need a lockfile for logic about output mergine
        let lockfile: Lockfile = self.lock(flox)?.into();

        let manifest = self.manifest(flox)?;

        // Resolve specs to modifications using manifest + lockfile.
        // This also handles PackageOnlyIncluded detection internally.
        let modifications = resolve_specs_to_modifications(&uninstall_specs, &manifest, &lockfile)?;

        let new_manifest = manifest.modify_packages(&modifications)?;
        let (store_path, _) = self.transact_with_manifest(&new_manifest, flox, out_link_prefix)?;

        // Collect the modified install ids that are still installed through includes
        let still_included = if let Some(compose) = &lockfile.compose {
            modifications
                .iter()
                .filter_map(|m| {
                    let include = compose.get_include_for_package(&m.install_id, &None).ok()?;
                    Some((m.install_id.clone(), include?))
                })
                .collect()
        } else {
            HashMap::new()
        };

        Ok(UninstallationAttempt {
            still_included,
            built_environment_store_paths: Some(store_path),
            modifications,
        })
    }

    /// Atomically edit this environment, ensuring that it still builds
    pub fn edit(
        &mut self,
        flox: &Flox,
        contents: String,
        out_link_prefix: Option<&Path>,
    ) -> Result<EditResult, EnvironmentError> {
        let maybe_up_to_date_lockfile = self.lockfile_if_up_to_date()?;

        // skip the edit if the contents are unchanged
        // and the existing lockfile is up to date
        // note: consumers of this function may call [Self::link] separately,
        //       causing an evaluation/build of the environment.
        let lockfile_is_up_to_date = maybe_up_to_date_lockfile.is_some();
        if self.contents_match_existing_manifest(&contents)? && lockfile_is_up_to_date {
            return Ok(EditResult::Unchanged);
        }

        let new_manifest = Manifest::parse_toml_typed(&contents)?;
        let (old_lockfile, migrated_manifest) =
            if let Some(lockfile) = maybe_up_to_date_lockfile.as_ref() {
                (
                    Some(lockfile.clone()),
                    new_manifest.migrate(Some(lockfile))?,
                )
            } else {
                let migrated = new_manifest.migrate(None)?;
                (None, migrated)
            };

        let (store_path, new_lockfile) =
            self.transact_with_manifest(&migrated_manifest, flox, out_link_prefix)?;

        Ok(EditResult::Changed {
            old_lockfile: Box::new(old_lockfile),
            new_lockfile: Box::new(new_lockfile),
            built_environment_store_paths: store_path,
        })
    }

    fn contents_match_existing_manifest(
        &self,
        contents: impl AsRef<str>,
    ) -> Result<bool, ManifestError> {
        Ok(self.manifest_without_migrating()?.contents_match(contents))
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
        // Intentionally does not acquire the transaction lock: edit_unsafe is a
        // migration path that uses the old temp-dir + replace_with model rather
        // than the in-place transaction model. Concurrent unsafe-edits can still
        // race, as they could before the transaction lock was introduced.

        // skip the edit if the contents are unchanged
        // note: consumers of this function may call [Self::link] separately,
        //       causing an evaluation/build of the environment.
        if self.contents_match_existing_manifest(&contents)? {
            return Ok(Ok(EditResult::Unchanged));
        }

        let new_manifest = Manifest::parse_toml_typed(&contents)?;
        let mut old_lockfile = self.lockfile_if_up_to_date()?;
        if old_lockfile.is_none() {
            // If locking fails, we still want to perform the unsafe edit, so
            // carry on with None as lockfile
            old_lockfile = self.lock(flox).ok().map(|lock_result| lock_result.into());
        }
        let migrated_manifest = new_manifest.migrate(old_lockfile.as_ref())?;

        let tempdir = tempfile::tempdir_in(&flox.temp_dir)
            .map_err(CoreEnvironmentError::MakeSandbox)?
            .keep();

        debug!(
            "transaction: making temporary environment in {}",
            tempdir.display()
        );
        let mut temp_env = self.writable(&tempdir)?;

        debug!("transaction: updating manifest");
        let maybe_original_schema = migrated_manifest.as_writable_maybe_in_original_schema()?;
        temp_env.update_manifest(&maybe_original_schema)?;

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

        let build_attempt = temp_env.build(flox, None);

        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;

        match build_attempt {
            Ok(store_path) => Ok(Ok(EditResult::Changed {
                old_lockfile: Box::new(old_lockfile),
                new_lockfile: Box::new(new_lockfile),
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
        out_link_prefix: Option<&Path>,
    ) -> Result<UpgradeResult, EnvironmentError> {
        tracing::debug!(to_upgrade = groups_or_iids.join(","), "upgrading");
        let manifest = self.manifest(flox)?;

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

            let store_path =
                self.transact_with_lockfile_contents(lockfile_contents, flox, out_link_prefix)?;
            result.store_path = Some(store_path);
        } else {
            let tmp_lockfile = tempfile::NamedTempFile::new_in(&flox.temp_dir)
                .map_err(CoreEnvironmentError::WriteLockfile)?;
            fs::write(&tmp_lockfile, lockfile_contents)
                .map_err(CoreEnvironmentError::WriteLockfile)?;

            // We are not interested in the store path here, so we ignore the result
            // Neither do we depend on services, so we pass `None`
            let auth = NixAuth::from_flox(flox).map_err(EnvironmentError::Auth)?;
            let _ = BuildEnvNix::new(auth)
                .build(&flox.catalog_client, tmp_lockfile.path(), None, None)
                .map_err(|e| EnvironmentError::Core(CoreEnvironmentError::BuildEnv(e)))?;
        }

        Ok(result)
    }

    fn ensure_valid_upgrade(
        groups_or_iids: &[&str],
        manifest: &Manifest<Migrated>,
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
        manifest: &Manifest<Migrated>,
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
            LockManifest::unlock_specified_packages_or_groups(&mut lockfile, groups_or_iids);
            lockfile
        });

        let upgraded_lockfile = LockManifest::lock_manifest(
            flox,
            &manifest.as_migrated_typed_only(),
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
        out_link_prefix: Option<&Path>,
    ) -> Result<UpgradeResult, EnvironmentError> {
        tracing::debug!(
            includes = to_upgrade.iter().join(","),
            "upgrading included environments"
        );

        let manifest = self.manifest(flox)?;

        let existing_lockfile_contents = self.existing_lockfile_contents()?;
        let existing_lockfile = existing_lockfile_contents
            .as_deref()
            .map(Lockfile::from_str)
            .transpose()?;

        let manifest_without_migrating = self.manifest_without_migrating()?;
        let original_schema = manifest_without_migrating.get_schema_version();

        // This is `mut` because we may need to update the on-disk manifest to match the
        // schema of the merged manifest, and then we'll also have to update the
        // `compose.composer` to match.
        let mut new_lockfile = LockManifest::lock_manifest_with_include_upgrades(
            flox,
            &manifest.as_migrated_typed_only(),
            existing_lockfile.as_ref(),
            &self.include_fetcher,
            Some(to_upgrade),
        )
        .block_on()?;

        // If the merged manifest required a newer schema than the composer's
        // on-disk manifest (due to composition introducing features like
        // explicit outputs), migrate the composer's manifest to match.
        self.ensure_manifest_schemas_match(original_schema, &mut new_lockfile)?;

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

        let store_path =
            self.transact_with_lockfile_contents(lockfile_contents, flox, out_link_prefix)?;
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

        // Never carry the transaction lock into the temporary copy: `replace_with`
        // moves this copy back over `env_dir`, which would re-instate a stale lock
        // and block every future transaction until it is deleted by hand.
        let _ = fs::remove_file(tempdir.as_ref().join(TRANSACTION_LOCK_FILENAME));

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

    /// Returns the canonical path to the transaction lock file for this environment.
    fn transaction_lock_path(&self) -> PathBuf {
        self.env_dir.join(TRANSACTION_LOCK_FILENAME)
    }

    /// Acquires an NFS-safe exclusive transaction lock on the environment directory.
    ///
    /// Uses the hard-link trick: writes a unique per-process claim file, then
    /// atomically hard-links it to the canonical lock path and verifies ownership
    /// by checking the link count.  `link(2)` is atomic on both local filesystems
    /// and NFS (where `open(O_CREAT|O_EXCL)` is not reliable across NFS clients).
    ///
    /// The lock file contains the PID, hostname, and username of the holder so
    /// that users sharing an environment over NFS can identify who is editing.
    ///
    /// Returns a guard that removes the lock when dropped.  Returns
    /// [`CoreEnvironmentError::TransactionLockHeld`] if another process holds
    /// the lock.
    fn acquire_transaction_lock(&self) -> Result<TransactionLock, CoreEnvironmentError> {
        use std::os::unix::fs::MetadataExt as _;

        use nix::unistd::{Uid, User, gethostname};

        let lock_path = self.transaction_lock_path();
        let pid = std::process::id();

        let hostname = gethostname()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "unknown".to_string());

        let username = User::from_uid(Uid::current())
            .ok()
            .flatten()
            .map(|u| u.name)
            .unwrap_or_else(|| format!("uid:{}", Uid::current()));

        let info = TransactionLockInfo {
            pid,
            hostname: hostname.clone(),
            username,
        };
        let info_json =
            serde_json::to_string_pretty(&info).expect("TransactionLockInfo is JSON-serializable");

        // One unique claim file per (host, process) so multiple hosts on NFS
        // can contend without clobbering each other's claim files.
        let claim_path = self
            .env_dir
            .join(format!("transaction.lock.{hostname}.{pid}"));
        fs::write(&claim_path, &info_json).map_err(CoreEnvironmentError::AcquireTransactionLock)?;

        // hard_link(claim, lock) is atomic on both local filesystems and NFS.
        // After the attempt we check nlink on claim_path: if it is 2, both
        // claim_path and lock_path reference the same inode, meaning we hold
        // the lock.  We check nlink rather than trusting the return value of
        // hard_link because some NFS servers return EEXIST even when the link
        // succeeded (a known NFS consistency quirk).
        let link_result = fs::hard_link(&claim_path, &lock_path);
        let nlink = fs::metadata(&claim_path).map(|m| m.nlink()).unwrap_or(0);
        let _ = fs::remove_file(&claim_path);

        if nlink >= 2 {
            // nlink is the authoritative signal: we own the lock.
            // Record the inode so Drop can verify ownership before removing.
            let inode = match fs::metadata(&lock_path) {
                Ok(m) => m.ino(),
                Err(e) => {
                    // Couldn't stat our own lock; remove it best-effort and fail.
                    let _ = fs::remove_file(&lock_path);
                    return Err(CoreEnvironmentError::AcquireTransactionLock(e));
                },
            };
            debug!(path = %lock_path.display(), "acquired transaction lock");
            Ok(TransactionLock { lock_path, inode })
        } else if let Err(e) = link_result {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                let owner = Self::read_lock_info(&lock_path);
                Err(CoreEnvironmentError::transaction_lock_held(
                    lock_path, owner,
                ))
            } else {
                // Genuine I/O failure (permission denied, read-only FS, etc.),
                // not a lock contention — surface it as AcquireTransactionLock.
                Err(CoreEnvironmentError::AcquireTransactionLock(e))
            }
        } else {
            // hard_link returned Ok but nlink < 2 — unexpected; treat conservatively
            // as lock held.
            let owner = Self::read_lock_info(&lock_path);
            Err(CoreEnvironmentError::transaction_lock_held(
                lock_path, owner,
            ))
        }
    }

    /// Read and parse the lock owner info from a transaction lock file.
    ///
    /// Returns `None` silently on any I/O or parse error.  Callers have already
    /// established that a lock contention exists; this is used only to enrich
    /// the error message with who holds the lock.
    fn read_lock_info(lock_path: &Path) -> Option<TransactionLockInfo> {
        fs::read_to_string(lock_path)
            .ok()
            .and_then(|s| serde_json::from_str::<TransactionLockInfo>(&s).ok())
    }

    /// Append `.bak` to `path`, returning the backup path.
    fn backup_path(path: &Path) -> PathBuf {
        let mut bak = path.to_owned();
        bak.add_extension("bak");
        bak
    }

    /// Remove a transaction backup after the transaction has committed.
    ///
    /// Removal is best-effort: the build has already succeeded and the real
    /// files are in their final state, so a failure here must not fail the
    /// transaction. A leftover backup would, however, make the next mutation
    /// fail with [`CoreEnvironmentError::PriorTransaction`], so warn with the
    /// path to let the user clear it by hand.
    fn remove_committed_backup(backup: &Path) {
        if let Err(err) = fs::remove_file(backup) {
            warn!(
                path = %backup.display(),
                %err,
                "failed to remove transaction backup after a successful transaction; \
                 delete it manually to unblock future edits"
            );
        }
    }

    /// Attempt to transactionally replace the manifest contents.
    ///
    /// The transaction is file-level rather than directory-level:
    /// - `manifest.toml` and `manifest.lock` are renamed aside to `*.bak`
    ///   before new versions are written.
    /// - `nix build` runs against the real environment path, so activation
    ///   out-link symlinks are written atomically as part of the build and are
    ///   never left pointing at a partially-committed state.
    /// - On build failure the backups are renamed back (preserving inode and
    ///   mtime); on success they are removed.
    #[must_use = "don't discard the store path of built environments"]
    fn transact_with_manifest(
        &mut self,
        manifest: &Manifest<Migrated>,
        flox: &Flox,
        out_link_prefix: Option<&Path>,
    ) -> Result<(BuildEnvOutputs, Lockfile), EnvironmentError> {
        let _lock = self
            .acquire_transaction_lock()
            .map_err(EnvironmentError::Core)?;

        debug!("transaction: validating services block");
        manifest.as_latest_schema().services.validate()?;

        debug!("transaction: locking environment");
        let existing_lockfile = self.existing_lockfile()?;
        let mut lockfile: Lockfile = self
            .lock_without_writing(flox, manifest, existing_lockfile.as_ref())?
            .into();

        debug!("transaction: ensuring manifest schemas match");
        if lockfile.manifest.get_schema_version() != manifest.original_schema()
            && let Some(compose) = lockfile.compose.as_mut()
        {
            compose.composer = manifest.as_migrated_typed_only().as_typed_only();
        }

        let writable_manifest =
            manifest.as_writable_maybe_in_original_schema_with_lockfile(Some(&lockfile))?;

        let manifest_path = self.manifest_path();
        let lockfile_path = self.lockfile_path();
        let manifest_bak = Self::backup_path(&manifest_path);
        let lockfile_bak = Self::backup_path(&lockfile_path);
        let lockfile_existed = lockfile_path.exists();

        if manifest_bak.exists() {
            return Err(EnvironmentError::Core(
                CoreEnvironmentError::PriorTransaction(manifest_bak),
            ));
        }
        if lockfile_bak.exists() {
            return Err(EnvironmentError::Core(
                CoreEnvironmentError::PriorTransaction(lockfile_bak),
            ));
        }

        debug!(
            manifest = %manifest_path.display(),
            lockfile = %lockfile_path.display(),
            "transaction: backing up manifest files"
        );
        fs::rename(&manifest_path, &manifest_bak)
            .map_err(|e| EnvironmentError::Core(CoreEnvironmentError::BackupTransaction(e)))?;
        if lockfile_existed && let Err(e) = fs::rename(&lockfile_path, &lockfile_bak) {
            let _ = fs::rename(&manifest_bak, &manifest_path);
            return Err(EnvironmentError::Core(
                CoreEnvironmentError::BackupTransaction(e),
            ));
        }

        debug!("transaction: writing new manifest and lockfile");
        let write_result: Result<(), EnvironmentError> = (|| {
            writable_manifest
                .write_to_file(&manifest_path)
                .map_err(|e| EnvironmentError::Core(CoreEnvironmentError::Manifest(e)))?;
            let mut lockfile_json =
                serde_json::to_string_pretty(&lockfile).expect("lockfile is valid JSON");
            lockfile_json.push('\n');
            write_atomically(&lockfile_path, lockfile_json).map_err(|e| {
                EnvironmentError::Core(CoreEnvironmentError::WriteLockfileAtomically(e))
            })?;
            Ok(())
        })();
        if let Err(e) = write_result {
            let _ = fs::remove_file(&manifest_path);
            let _ = fs::rename(&manifest_bak, &manifest_path);
            if lockfile_existed {
                let _ = fs::remove_file(&lockfile_path);
                let _ = fs::rename(&lockfile_bak, &lockfile_path);
            } else {
                // No prior lockfile: remove the newly-created one so the
                // transaction leaves no trace.
                let _ = fs::remove_file(&lockfile_path);
            }
            return Err(e);
        }

        debug!("transaction: building environment in-place");
        match self.build(flox, out_link_prefix) {
            Ok(store_path) => {
                debug!("transaction: removing backups");
                // The transaction has committed: the build succeeded and the new
                // manifest and lockfile are in place. Removing the backups is
                // best-effort cleanup — a failure here must neither fail the
                // transaction nor leave a backup that the guards above would later
                // mistake for an interrupted transaction. Warn so a genuinely
                // stuck backup can be cleared by hand.
                Self::remove_committed_backup(&manifest_bak);
                if lockfile_existed {
                    Self::remove_committed_backup(&lockfile_bak);
                }
                Ok((store_path, lockfile))
            },
            Err(build_err) => {
                debug!("transaction: build failed, restoring backups");
                let _ = fs::remove_file(&manifest_path);
                let _ = fs::rename(&manifest_bak, &manifest_path);
                if lockfile_existed {
                    let _ = fs::remove_file(&lockfile_path);
                    let _ = fs::rename(&lockfile_bak, &lockfile_path);
                } else {
                    // No prior lockfile: remove the newly-created one so the
                    // transaction leaves no trace.
                    let _ = fs::remove_file(&lockfile_path);
                }
                Err(EnvironmentError::Core(build_err))
            },
        }
    }

    /// Attempt to transactionally replace the lockfile contents.
    ///
    /// Uses the same file-level backup strategy as [`Self::transact_with_manifest`]:
    /// `manifest.lock` is renamed aside to `manifest.lock.bak`, the new
    /// lockfile is written in-place, the environment is built at its real path,
    /// and the backup is restored on build failure or removed on success.
    ///
    /// TODO: this is separate from transact_with_manifest because it
    /// shouldn't have to call lock. Currently build calls lock, but we
    /// shouldn't have to lock a second time.
    #[must_use = "don't discard the store path of built environments"]
    fn transact_with_lockfile_contents(
        &mut self,
        lockfile_contents: impl AsRef<str>,
        flox: &Flox,
        out_link_prefix: Option<&Path>,
    ) -> Result<BuildEnvOutputs, CoreEnvironmentError> {
        let _lock = self.acquire_transaction_lock()?;

        let lockfile_path = self.lockfile_path();
        let lockfile_bak = Self::backup_path(&lockfile_path);
        let lockfile_existed = lockfile_path.exists();

        if lockfile_bak.exists() {
            return Err(CoreEnvironmentError::PriorTransaction(lockfile_bak));
        }

        if lockfile_existed {
            debug!(lockfile = %lockfile_path.display(), "transaction: backing up lockfile");
            fs::rename(&lockfile_path, &lockfile_bak)
                .map_err(CoreEnvironmentError::BackupTransaction)?;
        }

        debug!("transaction: writing new lockfile");
        let mut contents_with_newline = lockfile_contents.as_ref().to_string();
        contents_with_newline.push('\n');
        if let Err(e) = write_atomically(&lockfile_path, contents_with_newline)
            .map_err(CoreEnvironmentError::WriteLockfileAtomically)
        {
            if lockfile_existed {
                let _ = fs::remove_file(&lockfile_path);
                let _ = fs::rename(&lockfile_bak, &lockfile_path);
            } else {
                let _ = fs::remove_file(&lockfile_path);
            }
            return Err(e);
        }

        debug!("transaction: building environment in-place");
        match self.build(flox, out_link_prefix) {
            Ok(store_path) => {
                debug!("transaction: removing lockfile backup");
                // Committed: best-effort cleanup only (see transact_with_manifest).
                if lockfile_existed {
                    Self::remove_committed_backup(&lockfile_bak);
                }
                Ok(store_path)
            },
            Err(build_err) => {
                debug!("transaction: build failed, restoring lockfile backup");
                if lockfile_existed {
                    let _ = fs::remove_file(&lockfile_path);
                    let _ = fs::rename(&lockfile_bak, &lockfile_path);
                } else {
                    let _ = fs::remove_file(&lockfile_path);
                }
                Err(build_err)
            },
        }
    }
}

/// A writable view of an environment directory
///
/// Typically within a temporary directory created by [CoreEnvironment::writable].
/// This is not public to enforce that environments are only edited atomically.
impl CoreEnvironment<ReadWrite> {
    /// Updates the environment manifest with the provided contents
    fn update_manifest(
        &mut self,
        manifest: &Manifest<Writable>,
    ) -> Result<(), CoreEnvironmentError> {
        debug!("writing new manifest to {}", self.manifest_path().display());
        manifest.write_to_file(self.manifest_path())?;
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
        old_lockfile: Box<Option<Lockfile>>,
        new_lockfile: Box<Lockfile>,
        built_environment_store_paths: BuildEnvOutputs,
    },
}

impl EditResult {
    /// The user needs to re-activate to have changes made to the environment
    /// take effect
    pub fn reactivate_required(&self) -> Result<bool, ManifestError> {
        match self {
            Self::Unchanged => Ok(false),
            Self::Changed {
                old_lockfile,
                new_lockfile,
                ..
            } => {
                let new_migrated = new_lockfile.migrated_manifest()?;
                let new_manifest = new_migrated.as_latest_schema();

                let old_migrated = old_lockfile
                    .as_ref()
                    .as_ref()
                    .map(|lockfile| lockfile.migrated_manifest())
                    .transpose()?;
                let old_manifest = old_migrated.as_ref().map(|m| m.as_latest_schema());

                let hook_changed =
                    old_manifest.and_then(|m| m.hook.as_ref()) != new_manifest.hook.as_ref();
                let vars_changed =
                    old_manifest.map(|m| m.vars.clone()).unwrap_or_default() != new_manifest.vars;
                let profile_changed =
                    old_manifest.and_then(|m| m.profile.as_ref()) != new_manifest.profile.as_ref();

                Ok(hook_changed || vars_changed || profile_changed)
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
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Lockfile(#[from] LockfileError),
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
    WriteLockfileAtomically(#[source] WriteError),
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

    #[error("could not acquire transaction lock")]
    AcquireTransactionLock(#[source] std::io::Error),
    /// Another process holds the transaction lock.
    /// The message is pre-formatted with PID/user/host from the lock file.
    #[error("{0}")]
    TransactionLockHeld(String),

    // endregion

    // region: mutable manifest errors
    #[error("could not open manifest")]
    OpenManifest(#[source] std::io::Error),
    #[error("could not write manifest")]
    UpdateManifest(#[source] std::io::Error),
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

    #[error("authentication error")]
    Auth(#[source] AuthError),
    // endregion
    #[error(transparent)]
    EnvError(#[from] Box<EnvironmentError>),
}

impl CoreEnvironmentError {
    fn transaction_lock_held(lock_path: PathBuf, owner: Option<TransactionLockInfo>) -> Self {
        let owner_desc = owner
            .map(|o| format!(" (pid {}, user {}, host {})", o.pid, o.username, o.hostname))
            .unwrap_or_default();
        CoreEnvironmentError::TransactionLockHeld(format!(
            "Another process is already editing this environment{owner_desc}.\n\
             Delete {} to proceed if the process is no longer running.",
            lock_path.display()
        ))
    }

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

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use flox_manifest::test_helpers::with_schema;
    use indoc::indoc;

    use super::*;
    use crate::flox::Flox;
    use crate::models::environment::fetcher::test_helpers::mock_include_fetcher;

    pub fn manifest_contents_with_incompatible_system(schema: KnownSchemaVersion) -> String {
        #[cfg(target_os = "macos")]
        let contents = with_schema(schema, indoc! {r#"
            [options]
            systems = [ "x86_64-linux" ]
        "#});
        #[cfg(target_os = "linux")]
        let contents = with_schema(schema, indoc! {r#"
            [options]
            systems = [ "aarch64-darwin" ]
        "#});
        contents
    }

    pub fn manifest_contents_with_latest_schema_and_incompatible_system() -> String {
        manifest_contents_with_incompatible_system(KnownSchemaVersion::latest())
    }

    pub fn manifest_contents_with_schema_and_incompatible_system(
        schema: KnownSchemaVersion,
    ) -> String {
        manifest_contents_with_incompatible_system(schema)
    }

    pub fn manifest_with_incompatible_system() -> Manifest<Validated> {
        Manifest::parse_toml_typed(manifest_contents_with_latest_schema_and_incompatible_system())
            .unwrap()
    }

    pub fn manifest_with_incompatible_system_v1() -> Manifest<Validated> {
        Manifest::parse_toml_typed(manifest_contents_with_schema_and_incompatible_system(
            KnownSchemaVersion::V1,
        ))
        .unwrap()
    }

    pub fn new_core_environment(flox: &Flox, contents: &str) -> CoreEnvironment {
        let env_path = tempfile::tempdir_in(&flox.temp_dir).unwrap().keep();
        fs::write(env_path.join(MANIFEST_FILENAME), contents).unwrap();

        CoreEnvironment::new(&env_path, mock_include_fetcher())
    }

    pub fn new_core_environment_with_lockfile(
        flox: &Flox,
        manifest_contents: &str,
        lockfile_contents: &str,
    ) -> CoreEnvironment {
        let env_path = tempfile::tempdir_in(&flox.temp_dir).unwrap().keep();
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
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    use flox_core::activate::mode::ActivateMode;
    use flox_manifest::interfaces::AsLatestSchema;
    use flox_manifest::parsed::Inner;
    use flox_manifest::raw::CatalogPackage;
    use flox_manifest::test_helpers::{with_latest_schema, with_schema};
    use flox_test_utils::{GENERATED_DATA, MANUALLY_GENERATED};
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use tempfile::{TempDir, tempdir_in};
    use test_helpers::{new_core_environment_from_env_files, new_core_environment_with_lockfile};

    use self::test_helpers::new_core_environment;
    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::environment::test_helpers::manifest_with_incompatible_system;
    use crate::providers::catalog::test_helpers::catalog_replay_client;
    use crate::providers::services::process_compose::SERVICE_CONFIG_FILENAME;
    use crate::utils::serialize_json_with_newline;

    /// Create a CoreEnvironment with an empty manifest (with version = 1)
    fn empty_core_environment() -> (CoreEnvironment, Flox, TempDir) {
        let (flox, tempdir) = flox_instance();

        (new_core_environment(&flox, "version = 1"), flox, tempdir)
    }

    /// Check that `edit` updates the manifest and creates a lockfile
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "impure-unit-tests")]
    async fn edit_env_creates_manifest_and_lockfile() {
        use crate::providers::catalog::test_helpers::catalog_replay_client;

        let (mut flox, tempdir) = flox_instance();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        fs::write(
            env_path.path().join(MANIFEST_FILENAME),
            with_latest_schema(""),
        )
        .unwrap();

        let mut env_view = CoreEnvironment::new(&env_path, IncludeFetcher {
            base_directory: None,
        });

        let new_env_str = with_latest_schema(indoc! {r#"
            [install]
            hello.pkg-path = "hello"
        "#});

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;
        env_view.edit(&flox, new_env_str.to_string(), None).unwrap();

        assert_eq!(
            env_view
                .manifest_without_migrating()
                .unwrap()
                .as_writable()
                .to_string(),
            new_env_str
        );
        assert!(env_view.env_dir.join(LOCKFILE_FILENAME).exists());
    }

    /// A no-op with edit against a locked environment returns EditResult::Unchanged
    #[test]
    fn edit_no_op_locked_returns_unchanged() {
        let (flox, _temp_dir_handle) = flox_instance();

        let same_manifest = with_latest_schema("");
        let mut env_view = new_core_environment(&flox, &same_manifest);
        env_view.lock(&flox).unwrap(); // Explicit lock

        let result = env_view.edit(&flox, same_manifest, None).unwrap();
        assert_eq!(result, EditResult::Unchanged);
    }

    /// A no-op with edit against a locked environment with an old schema version
    /// returns EditResult::Unchanged
    #[test]
    fn edit_no_op_locked_old_schema_returns_unchanged() {
        let (flox, _temp_dir_handle) = flox_instance();

        let same_manifest = "version = 1";
        let mut env_view = new_core_environment(&flox, same_manifest);
        env_view.lock(&flox).unwrap(); // Explicit lock

        let result = env_view
            .edit(&flox, same_manifest.to_string(), None)
            .unwrap();
        assert_eq!(result, EditResult::Unchanged);
    }

    /// A no-op with edit against an unlocked environment returns EditResult::Changed
    #[test]
    fn edit_no_op_unlocked_returns_changed() {
        let (flox, _temp_dir_handle) = flox_instance();

        let same_manifest = "version = 1";
        let mut env_view = new_core_environment(&flox, same_manifest);

        let result = env_view
            .edit(&flox, same_manifest.to_string(), None)
            .unwrap();
        assert!(matches!(result, EditResult::Changed { .. }));
    }

    /// Trying to build a manifest with a system other than the current one
    /// results in an error that is_incompatible_system_error()
    #[test]
    fn build_incompatible_system() {
        let (flox, _temp_dir_handle) = flox_instance();
        let mut env_view = new_core_environment(&flox, "");
        let mut temp_env = env_view
            .writable(tempdir_in(&flox.temp_dir).unwrap().keep())
            .unwrap();
        let manifest = manifest_with_incompatible_system().migrate(None).unwrap();
        let maybe_original_schema = manifest.as_writable_maybe_in_original_schema().unwrap();
        temp_env.update_manifest(&maybe_original_schema).unwrap();
        temp_env.lock(&flox).unwrap();
        env_view.replace_with(temp_env).unwrap();

        let result = env_view.build(&flox, None).unwrap_err();

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
        let store_path = env.build(&flox, None).unwrap();
        let config_path = store_path.dev.join(SERVICE_CONFIG_FILENAME);
        assert!(config_path.exists());
    }

    /// Installing hello with edit returns EditResult::Changed and
    /// reactivate_required() returns false
    #[tokio::test(flavor = "multi_thread")]
    async fn edit_adding_package_returns_changed() {
        let (mut env_view, mut flox, _temp_dir_handle) = empty_core_environment();

        let new_env_str = r#"
        version = 1

        [install]
        hello.pkg-path = "hello"
        "#;

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;
        let result = env_view.edit(&flox, new_env_str.to_string(), None).unwrap();

        assert!(matches!(result, EditResult::Changed { .. }));
        assert!(!result.reactivate_required().unwrap());
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

        let result = env_view.edit(&flox, new_env_str.to_string(), None).unwrap();

        assert!(result.reactivate_required().unwrap());
    }

    /// Check that with an empty list of packages to upgrade, all packages are upgraded
    #[tokio::test(flavor = "multi_thread")]
    async fn upgrade_with_empty_list_upgrades_all() {
        let (mut env_view, mut flox, _temp_dir_handle) = empty_core_environment();

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/old_hello.yaml")).await;
        env_view
            .install(
                &[PackageToInstall::Catalog(
                    CatalogPackage::from_str("hello").unwrap(),
                )],
                &flox,
                None,
            )
            .unwrap();

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;

        let manifest = env_view.manifest(&flox).unwrap();
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

    /// A pre-existing transaction lock file causes `TransactionLockHeld`
    #[test]
    fn detects_concurrent_transaction() {
        let (_flox, tempdir) = flox_instance();
        let env_path = tempfile::tempdir_in(&tempdir).unwrap();

        let lock_info = TransactionLockInfo {
            pid: 99999,
            hostname: "other-host".to_string(),
            username: "alice".to_string(),
        };
        let lock_path = env_path.path().join("transaction.lock");
        fs::write(
            &lock_path,
            serde_json::to_string_pretty(&lock_info).unwrap(),
        )
        .unwrap();

        let env_view = CoreEnvironment::new(&env_path, IncludeFetcher {
            base_directory: None,
        });
        let err = env_view
            .acquire_transaction_lock()
            .expect_err("should fail when lock file exists");

        assert!(
            matches!(err, CoreEnvironmentError::TransactionLockHeld(_)),
            "unexpected error: {err}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("pid 99999"),
            "message should include pid: {msg}"
        );
        assert!(
            msg.contains("alice"),
            "message should include username: {msg}"
        );
        assert!(
            msg.contains("other-host"),
            "message should include hostname: {msg}"
        );
    }

    /// The transaction lock file is removed after a successful lock acquisition
    #[test]
    fn transaction_lock_released_on_drop() {
        let (_flox, tempdir) = flox_instance();
        let env_path = tempfile::tempdir_in(&tempdir).unwrap();

        let env_view = CoreEnvironment::new(&env_path, IncludeFetcher {
            base_directory: None,
        });
        let lock_path = env_view.transaction_lock_path();

        let guard = env_view
            .acquire_transaction_lock()
            .expect("should acquire lock");
        assert!(lock_path.exists(), "lock file should exist while held");

        drop(guard);
        assert!(
            !lock_path.exists(),
            "lock file should be removed after drop"
        );
    }

    /// `writable` must not copy the transaction lock into the temporary
    /// environment: otherwise `replace_with` would move a stale lock back into
    /// the environment directory and block every future transaction.
    #[test]
    fn writable_strips_transaction_lock() {
        let (_flox, tempdir) = flox_instance();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        let sandbox_path = tempfile::tempdir_in(&tempdir).unwrap();

        let mut env_view = CoreEnvironment::new(&env_path, IncludeFetcher {
            base_directory: None,
        });

        // Simulate a stranded lock left in the environment directory.
        fs::write(env_view.transaction_lock_path(), "{}").unwrap();

        let temp_env = env_view.writable(&sandbox_path).unwrap();
        assert!(
            !sandbox_path.path().join(TRANSACTION_LOCK_FILENAME).exists(),
            "writable copy must not contain the transaction lock"
        );

        env_view.replace_with(temp_env).unwrap();
        assert!(
            !env_path.path().join(TRANSACTION_LOCK_FILENAME).exists(),
            "replace_with must not re-instate the stale transaction lock"
        );
    }

    /// Removing a committed backup is best-effort: it deletes an existing
    /// backup and is a no-op when the backup is already gone, so a successful
    /// transaction is never turned into a failure or left blocking the next one.
    #[test]
    fn remove_committed_backup_is_best_effort() {
        let (_flox, tempdir) = flox_instance();
        let dir = tempfile::tempdir_in(&tempdir).unwrap();
        let backup = dir.path().join("manifest.toml.bak");

        fs::write(&backup, "stale").unwrap();
        CoreEnvironment::<ReadOnly>::remove_committed_backup(&backup);
        assert!(!backup.exists(), "existing backup should be removed");

        // Already absent: must not panic.
        CoreEnvironment::<ReadOnly>::remove_committed_backup(&backup);
        assert!(!backup.exists());
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
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "impure-unit-tests")]
    async fn build_flox_environment_and_links() {
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

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;

        let out_link = env_path.path().with_extension("out-link");
        std::fs::create_dir_all(&out_link).expect("create out-link dir");
        let out_link = CanonicalPath::new(&out_link).expect("canonicalize out-link dir");

        env_view.lock(&flox).expect("locking should succeed");
        // Use a prefix so nix writes <prefix>-dev and <prefix>-run symlinks.
        let prefix = out_link.join("env");
        env_view
            .build(&flox, Some(&prefix))
            .expect("build should succeed");

        // very rudimentary check that the environment manifest built correctly
        // and linked to the out-link.
        assert!(out_link.join("env-dev").join("bin/hello").exists());
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

    /// Locking an environment should not write a lockfile if the contents are
    /// semantically equivalent to the existing lockfile.
    #[test]
    fn lock_skips_write_if_formatting_changes() {
        let (flox, _temp_dir_handle) = flox_instance();
        let mut environment =
            new_core_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        // add some whitespace to the file
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

        assert_eq!(mtime_after, mtime_original);
    }

    /// Locking an environment should write a lockfile if the contents change
    /// semantically compared to the existing lockfile
    #[test]
    fn lock_writes_if_modified() {
        let (flox, _temp_dir_handle) = flox_instance();
        let mut environment =
            new_core_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        // Make a non-formatting change to the lock
        {
            let mut lockfile = environment.existing_lockfile().unwrap().unwrap();
            let mut manifest = lockfile.migrated_manifest().unwrap();
            manifest.as_latest_schema_mut().options.activate.mode = Some(ActivateMode::Dev);
            lockfile.manifest = manifest.into();
            let lockfile_contents = serialize_json_with_newline(&lockfile).unwrap();
            let lockfile_path = environment.lockfile_path();
            let mut file = OpenOptions::new().write(true).open(lockfile_path).unwrap();
            file.write_all(lockfile_contents.as_bytes()).unwrap();

            // fsync metadata to ensure the mtime is updated
            file.sync_all().unwrap();
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

    #[test]
    fn edit_fails_when_daemon_has_no_shutdown_command() {
        let (flox, _dir) = flox_instance();
        let initial_manifest = with_latest_schema("");
        let mut env = new_core_environment(&flox, &initial_manifest);
        let bad_manifest = with_latest_schema(indoc! {r#"
            [services.bad]
            command = "cmd"
            is-daemon = true
            shutdown.command = "cmd" # we're going to delete this
        "#});
        let mut manifest = Manifest::parse_and_migrate(bad_manifest, None).unwrap();
        manifest
            .as_latest_schema_mut()
            .services
            .inner_mut()
            .get_mut("bad")
            .unwrap()
            .shutdown = None;
        let res = env.transact_with_manifest(&manifest, &flox, None);
        assert!(matches!(
            res,
            Err(EnvironmentError::ManifestError(
                ManifestError::InvalidServiceConfig(_)
            ))
        ));
    }

    #[test]
    fn lock_doesnt_migrate_backwards_compatible_manifest() {
        let (flox, tmpdir) = flox_instance();
        let manifest_contents = with_schema(KnownSchemaVersion::V1, "");
        let original_manifest = Manifest::parse_toml_typed(&manifest_contents).unwrap();
        let mut env = new_core_environment(&flox, &manifest_contents);
        let mut writable_env = env.writable(tmpdir.path()).unwrap();
        writable_env
            .update_manifest(&original_manifest.as_writable())
            .unwrap();
        writable_env.lock(&flox).unwrap();
        let post_lock_manifest = writable_env.manifest_without_migrating().unwrap();
        assert_eq!(
            original_manifest.as_writable().to_string(),
            post_lock_manifest.as_writable().to_string()
        );
    }
}
