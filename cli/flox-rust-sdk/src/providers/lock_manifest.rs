use std::path::PathBuf;

use crate::models::environment::EnvironmentError;
use crate::providers::catalog;
use crate::providers::flake_installable_locker::FlakeInstallableError;

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("failed to resolve packages")]
    CatalogResolve(#[from] catalog::ResolveError),

    // todo: this should probably part of some validation logic of the manifest file
    //       rather than occurring during the locking process creation
    #[error("unrecognized system type: {0}")]
    UnrecognizedSystem(String),

    #[error("resolution failed: {0}")]
    ResolutionFailed(ResolutionFailures),

    // todo: this should probably part of some validation logic of the manifest file
    //       rather than occurring during the locking process creation
    #[error(
        "'{install_id}' specifies disabled or unknown system '{system}' (enabled systems: {enabled_systems})",
        enabled_systems=enabled_systems.join(", ")
    )]
    SystemUnavailableInManifest {
        install_id: String,
        system: String,
        enabled_systems: Vec<String>,
    },

    #[error(
        "The package '{0}' has license '{1}' which is not in the list of allowed licenses.\n\nAllow this license by adding it to 'options.allow.licenses' in manifest.toml"
    )]
    LicenseNotAllowed(String, String),
    #[error(
        "The package '{0}' is marked as broken.\n\nAllow broken packages by setting 'options.allow.broken = true' in manifest.toml"
    )]
    BrokenNotAllowed(String),
    #[error(
        "The package '{0}' has an unfree license.\n\nAllow unfree packages by setting 'options.allow.unfree = true' in manifest.toml"
    )]
    UnfreeNotAllowed(String),

    #[error("Corrupt manifest; couldn't find flake package descriptor for locked install_id '{0}'")]
    MissingPackageDescriptor(String),

    #[error(transparent)]
    LockFlakeNixError(FlakeInstallableError),
    #[error("catalog returned install id not in manifest: {0}")]
    InstallIdNotInManifest(String),
}

/// Errors that occur during merging a manifest that flox edit can recover from
#[derive(Debug, thiserror::Error)]
pub enum RecoverableMergeError {
    #[error(transparent)]
    Merge(MergeError),

    #[error("failed to fetch environment '{include}'")]
    Fetch {
        include: IncludeDescriptor,
        #[source]
        err: Box<EnvironmentError>,
    },

    #[error(
        "cannot include environment since its manifest and lockfile are out of sync\n\
         \n\
         To resolve this issue run 'flox edit -d {0}' and retry\n"
    )]
    PathOutOfSync(PathBuf),

    #[error(
        "cannot include environment since it has changes not yet synced to a generation.\n\
         \n\
         To resolve this issue, run either\n\
         * 'flox edit -d {0} --sync' to commit your local changes to a new generation\n\
         * 'flox edit -d {0} --reset' to discard your local changes and reset to the latest generation\n"
    )]
    ManagedOutOfSync(PathBuf),

    /// Use this error when we don't need an error type to reuse in multiple places
    #[error("{0}")]
    Catchall(String),

    #[error("remote environments cannot include local environments")]
    RemoteCannotIncludeLocal,
}

#[derive(Debug, Clone)]
pub enum LockResult {
    /// Locking produced a new Lockfile.
    /// The change could be a minimal as whitespace.
    Changed(Lockfile),
    /// Locking did not produce a new Lockfile.
    Unchanged(Lockfile),
}

impl From<LockResult> for Lockfile {
    fn from(result: LockResult) -> Self {
        match result {
            LockResult::Changed(lockfile) => lockfile,
            LockResult::Unchanged(lockfile) => lockfile,
        }
    }
}

struct LockManifest;

impl LockManifest {
    /// Merge included environments, resolve the merged manifest, and return the resulting lockfile
    ///
    /// Already resolved packages will not be re-resolved,
    /// and already fetched includes will not be re-fetched.
    pub async fn lock_manifest(
        flox: &Flox,
        manifest: &Manifest,
        seed_lockfile: Option<&Lockfile>,
        include_fetcher: &IncludeFetcher,
    ) -> Result<Lockfile, EnvironmentError> {
        Self::lock_manifest_with_include_upgrades(
            flox,
            manifest,
            seed_lockfile,
            include_fetcher,
            None,
        )
        .await
    }

    /// Lock, upgrading the specified included environments if to_upgrade is
    /// Some
    ///
    /// If to_upgrade is an empty vector, all included environments are
    /// re-fetched.
    /// If to_upgrade is None, only included environments not in the seed lockfile
    /// are fetched.
    pub async fn lock_manifest_with_include_upgrades(
        flox: &Flox,
        manifest: &Manifest,
        seed_lockfile: Option<&Lockfile>,
        include_fetcher: &IncludeFetcher,
        to_upgrade: Option<Vec<String>>,
    ) -> Result<Lockfile, EnvironmentError> {
        let (merged, compose) = Self::merge_manifest(
            flox,
            manifest,
            seed_lockfile,
            include_fetcher,
            ManifestMerger::Shallow(ShallowMerger),
            to_upgrade,
        )
        .map_err(EnvironmentError::Recoverable)?;
        let packages = Self::resolve_manifest(
            &merged,
            seed_lockfile,
            &flox.catalog_client,
            &flox.installable_locker,
        )
        .await
        .map_err(|e| EnvironmentError::Core(CoreEnvironmentError::Resolve(e)))?;
        let lockfile = Lockfile {
            version: Version::<1>,
            manifest: merged,
            packages,
            compose,
        };

        Ok(lockfile)
    }

    /// Fetch included environments and merge them with the manifest, returning
    /// the merged manifest and a Compose object with the contents of all fetched includes.
    ///
    /// If the manifest does not include any environments, None is returned
    /// instead of a Compose object.
    ///
    /// Any included environments already in the seed lockfile will not be
    /// re-fetched, unless they are in to_upgrade.
    ///
    /// All environments in to_upgrade will be re-fetched, and an error is
    /// returned if any environments in to_upgrade do not refer to an actual include.
    ///
    /// If to_upgrade is an empty vector, all included environments are
    /// re-fetched.
    /// If to_upgrade is None, only included environments not in the seed lockfile
    /// are fetched.
    #[instrument(skip_all, fields(progress = "Composing environments"))]
    fn merge_manifest(
        flox: &Flox,
        manifest: &Manifest,
        seed_lockfile: Option<&Lockfile>,
        include_fetcher: &IncludeFetcher,
        merger: ManifestMerger,
        mut to_upgrade: Option<Vec<String>>,
    ) -> Result<(Manifest, Option<Compose>), RecoverableMergeError> {
        if manifest.include.environments.is_empty() {
            if to_upgrade.is_some() {
                return Err(RecoverableMergeError::Catchall(
                    "environment has no included environments".to_string(),
                ));
            }
            return Ok((manifest.clone(), None));
        }

        debug!("composing included environments");

        // Fetch included manifests we don't already have in seed_lockfile.
        // Note that we have to preserve the order of the includes in the
        // manifest.
        let mut locked_includes: Vec<LockedInclude> = vec![];
        let upgrade_all = to_upgrade
            .as_ref()
            .map(|to_upgrade| to_upgrade.is_empty())
            .unwrap_or(false);
        for include_environment in &manifest.include.environments {
            debug!(
                name = include_environment.to_string(),
                "inspecting included environment"
            );
            let existing_locked_include = 'existing: {
                // Don't use existing locked includes if we're upgradeing all
                // includes
                if upgrade_all {
                    break 'existing None;
                }

                // If there's a seed_lockfile
                let Some(seed_lockfile) = seed_lockfile else {
                    break 'existing None;
                };
                // And the seed lockfile was generated from a manifest with includes
                let Some(compose) = &seed_lockfile.compose else {
                    break 'existing None;
                };
                // And we can find an identical include descriptor in the seed lockfile
                // Then use the existing locked include
                compose
                    .include
                    .iter()
                    .find(|locked_include| &locked_include.descriptor == include_environment)
                    .cloned()
            };

            let locked_include = match existing_locked_include {
                Some(locked_include) => {
                    debug!("found existing locked include for {include_environment}");
                    // The following is a weird edge case,
                    // but I don't think it's too much of a problem:
                    // Suppose composer includes ./dir1 which has name A in
                    // env.json
                    // ./dir1 gets renamed A -> B
                    // A manual edit includes ./dir2 which has name A in
                    // env.json
                    // If we upgrade name A, if we loop over ./dir1 first, we'll
                    // fetch both ./dir1 and ./dir2.
                    // If we loop over ./dir2 first, we'll only fetch ./dir2.

                    // Check if the existing locked include needs to be upgraded
                    // If it does, remove it from to_upgrade to keep track of
                    // which includes have been upgraded.
                    let should_refetch = to_upgrade
                        .as_mut()
                        .map(|to_upgrade| {
                            Self::remove_matching_include(to_upgrade, &locked_include)
                        })
                        .unwrap_or(false);

                    if should_refetch {
                        debug!(
                            name = include_environment.to_string(),
                            "upgrading included environment"
                        );
                        include_fetcher
                            .fetch(flox, include_environment)
                            .map_err(|e| RecoverableMergeError::Fetch {
                                include: include_environment.clone(),
                                err: Box::new(e),
                            })?
                    } else {
                        debug!(
                            name = include_environment.to_string(),
                            "using existing locked include from lockfile"
                        );

                        locked_include
                    }
                },
                None => {
                    debug!(
                        name = include_environment.to_string(),
                        "fetching included environment"
                    );

                    let locked_include =
                        include_fetcher
                            .fetch(flox, include_environment)
                            .map_err(|e| RecoverableMergeError::Fetch {
                                include: include_environment.clone(),
                                err: Box::new(e),
                            })?;
                    // If this include needed to be upgraded, remove from
                    // to_upgrade to keep track that it was
                    if let Some(to_upgrade) = &mut to_upgrade {
                        Self::remove_matching_include(to_upgrade, &locked_include);
                    }
                    locked_include
                },
            };
            locked_includes.push(locked_include);
        }

        Self::check_locked_names_unique(&locked_includes)?;

        if let Some(to_upgrade) = &to_upgrade
            && let Some(unused_include_to_upgrade) = to_upgrade.first()
        {
            return Err(RecoverableMergeError::Catchall(format!(
                "unknown included environment to check for changes '{}'",
                unused_include_to_upgrade
            )));
        }

        // Call the merger with all the manifests
        let composite = CompositeManifest {
            composer: manifest.clone(),
            deps: locked_includes
                .iter()
                .map(|include| (include.name.clone(), include.manifest.clone()))
                .collect(),
        };

        let (merged, warnings) = composite
            .merge_all(merger)
            .map_err(RecoverableMergeError::Merge)?;

        // Stitch everything together into a Compose object
        let compose = Compose {
            composer: manifest.clone(),
            include: locked_includes,
            warnings,
        };

        Ok((merged, Some(compose)))
    }

    /// Helper method that removes the first IncludeToUpgrade that matches a given
    /// LockedInclude.
    /// Used to keep track of what includes have been upgraded.
    fn remove_matching_include(
        to_upgrade: &mut Vec<String>,
        locked_include: &LockedInclude,
    ) -> bool {
        let position = to_upgrade
            .iter()
            .position(|name| &locked_include.name == name);
        match position {
            Some(position) => {
                to_upgrade.swap_remove(position);
                true
            },
            None => false,
        }
    }

    /// Check that all names in a list of locked includes are unique
    fn check_locked_names_unique(
        locked_includes: &[LockedInclude],
    ) -> Result<(), RecoverableMergeError> {
        let mut seen_names = HashSet::new();
        for locked_include in locked_includes {
            if !seen_names.insert(&locked_include.name) {
                return Err(RecoverableMergeError::Catchall(formatdoc! {
                "multiple environments in include.environments have the name '{}'
                 A unique name can be provided with the 'name' field.", locked_include.name}));
            }
        }
        Ok(())
    }

    /// Resolve packages for a given manifest
    ///
    /// Uses the catalog service to resolve [ManifestPackageDescriptorCatalog],
    /// and an [InstallableLocker] to lock [ManifestPackageDescriptorFlake] descriptors.
    ///
    /// If a seed lockfile is provided, packages that are already locked
    /// will constrain the resolution of catalog packages to the same derivation.
    /// Already locked flake installables will not be locked again,
    /// and copied from the seed lockfile as is.
    ///
    /// Catalog and flake installables are locked separately, using largely symmetric logic.
    /// Keeping the locking of each kind separate keeps the existing methods simpler
    /// and allows for potential parallelization in the future.
    #[instrument(skip_all, fields(progress = "Locking environment"))]
    async fn resolve_manifest(
        manifest: &Manifest,
        seed_lockfile: Option<&Lockfile>,
        client: &impl catalog::ClientTrait,
        installable_locker: &impl InstallableLocker,
    ) -> Result<Vec<LockedPackage>, ResolveError> {
        let catalog_groups = Self::collect_package_groups(manifest, seed_lockfile)?;
        let (mut already_locked_packages, groups_to_lock) =
            Self::split_fully_locked_groups(catalog_groups, seed_lockfile);

        let flake_installables = Self::collect_flake_installables(manifest);
        let (already_locked_installables, installables_to_lock) =
            Self::split_locked_flake_installables(flake_installables, seed_lockfile);

        // Store paths are locked by definition
        let locked_store_paths = Self::collect_store_paths(manifest)
            .into_iter()
            .map(LockedPackage::StorePath)
            .collect();

        // The manifest could have been edited since locking packages,
        // in which case there may be packages that aren't allowed.
        Self::check_packages_are_allowed(
            already_locked_packages
                .iter()
                .filter_map(LockedPackage::as_catalog_package_ref),
            &manifest.options.allow,
        )?;

        // Update the priority of already locked packages to match the manifest.
        Self::update_priority(&mut already_locked_packages, manifest);

        if groups_to_lock.is_empty() && installables_to_lock.is_empty() {
            debug!("All packages are already locked, skipping resolution");
            return Ok([
                locked_store_paths,
                already_locked_packages,
                already_locked_installables,
            ]
            .concat());
        }

        // lock packages
        let resolved = if !groups_to_lock.is_empty() {
            client
                .resolve(groups_to_lock)
                .await
                .map_err(ResolveError::CatalogResolve)?
        } else {
            vec![]
        };

        // unpack locked packages from response
        let locked_packages: Vec<LockedPackage> =
            Self::locked_packages_from_resolution(manifest, resolved)?
                .map(Into::into)
                .collect();

        let locked_installables = if !installables_to_lock.is_empty() {
            Self::lock_flake_installables(installable_locker, installables_to_lock)?
                .map(Into::into)
                .collect()
        } else {
            vec![]
        };

        // The server should be checking this,
        // but double check
        Self::check_packages_are_allowed(
            locked_packages
                .iter()
                .filter_map(LockedPackage::as_catalog_package_ref),
            &manifest.options.allow,
        )?;

        Ok([
            locked_store_paths,
            already_locked_packages,
            locked_packages,
            already_locked_installables,
            locked_installables,
        ]
        .concat())
    }

    /// Given locked packages and manifest options allows, verify that the
    /// locked packages are allowed.
    fn check_packages_are_allowed<'a>(
        locked_packages: impl IntoIterator<Item = &'a LockedPackageCatalog>,
        allow: &Allows,
    ) -> Result<(), ResolveError> {
        for package in locked_packages {
            if let Some(ref licenses) = allow.licenses {
                // If licenses is empty, allow any license.
                // There isn't any reason to disallow all licenses,
                // and setting licenses to [] is the only way with composition
                // currently to allow all licenses if an included environment has licenses.
                if !licenses.is_empty() {
                    let Some(ref license) = package.license else {
                        continue;
                    };

                    if !licenses.iter().any(|allowed| allowed == license) {
                        return Err(ResolveError::LicenseNotAllowed(
                            package.install_id.to_string(),
                            license.to_string(),
                        ));
                    }
                }
            }

            // Don't allow broken by default
            if !allow.broken.unwrap_or(false) {
                // Assume a package isn't broken
                if package.broken.unwrap_or(false) {
                    return Err(ResolveError::BrokenNotAllowed(
                        package.install_id.to_owned(),
                    ));
                }
            }

            // Allow unfree by default
            if !allow.unfree.unwrap_or(true) {
                // Assume a package isn't unfree
                if package.unfree.unwrap_or(false) {
                    return Err(ResolveError::UnfreeNotAllowed(
                        package.install_id.to_owned(),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Update the priority of already locked packages to match the manifest.
    ///
    /// The `priority` field is originally set when constructing in [LockedPackageCatalog::from_parts],
    /// after resolution.
    /// Already locked packages are not re-resolved for priority changes
    /// as priority is not a constraint for resolution.
    /// The priority in the manifest may have changed since the package was locked,
    /// so we update the priority of already locked packages to match the manifest.
    fn update_priority<'a>(
        already_locked_packages: impl IntoIterator<Item = &'a mut LockedPackage>,
        manifest: &Manifest,
    ) {
        for locked_package in already_locked_packages {
            let LockedPackage::Catalog(LockedPackageCatalog {
                install_id,
                priority,
                ..
            }) = locked_package
            else {
                // `already_locked_packages`` should only contain catalog packages to begin with
                // and locked flake installables do not have a priority (yet?),
                // so this shouldn't occur.
                return;
            };

            let new_priority = manifest
                .install
                .inner()
                .get(install_id)
                .and_then(|descriptor| descriptor.as_catalog_descriptor_ref())
                .and_then(|descriptor| descriptor.priority)
                .unwrap_or(DEFAULT_PRIORITY);

            *priority = new_priority;
        }
    }

    /// Transform a lockfile into a mapping that is easier to query:
    /// Lockfile -> { (install_id, system): (package_descriptor, locked_package) }
    fn make_seed_mapping(
        seed: &Lockfile,
    ) -> HashMap<(&str, &str), (&ManifestPackageDescriptor, &LockedPackage)> {
        seed.packages
            .iter()
            .filter_map(|locked| {
                let system = locked.system().as_str();
                let install_id = locked.install_id();
                let descriptor = seed.manifest.install.inner().get(locked.install_id())?;
                Some(((install_id, system), (descriptor, locked)))
            })
            .collect()
    }

    /// Creates package groups from a flat map of (catalog) install descriptors
    ///
    /// A group is created for each unique combination of (`descriptor.package_group` ｘ `descriptor.systems``).
    /// If descriptor.systems is [None], a group with `default_system` is created for each `package_group`.
    /// Each group contains a list of package descriptors that belong to that group.
    ///
    /// `seed_lockfile` is used to provide existing derivations for packages that are already locked,
    /// e.g. by a previous lockfile.
    /// These packages are used to constrain the resolution.
    /// If a package in `manifest` does not have a corresponding package in `seed_lockfile`,
    /// that package will be unconstrained, allowing a first install.
    ///
    /// As package groups only apply to catalog descriptors,
    /// this function **ignores other [ManifestPackageDescriptor] variants**.
    /// Those are expected to be locked separately.
    ///
    /// Greenkeeping: this function seem to return a [Result]
    /// only due to parsing [System] strings to [PackageSystem].
    /// If we restricted systems earlier with a common `System` type,
    /// fallible conversions like that would be unnecessary,
    /// or would be pushed higher up.
    fn collect_package_groups(
        manifest: &Manifest,
        seed_lockfile: Option<&Lockfile>,
    ) -> Result<impl Iterator<Item = PackageGroup>, ResolveError> {
        let seed_locked_packages = seed_lockfile.map_or_else(HashMap::new, Self::make_seed_mapping);

        // Using a btree map to ensure consistent ordering
        let mut map = BTreeMap::new();

        let manifest_systems = manifest.options.systems.as_deref();

        let maybe_licenses = manifest
            .options
            .allow
            .licenses
            .clone()
            .and_then(|licenses| {
                if licenses.is_empty() {
                    None
                } else {
                    Some(licenses)
                }
            });

        for (install_id, manifest_descriptor) in manifest.install.inner().iter() {
            // package groups are only relevant to catalog descriptors
            let Some(manifest_descriptor) = manifest_descriptor.as_catalog_descriptor_ref() else {
                continue;
            };

            let resolved_descriptor_base = PackageDescriptor {
                install_id: install_id.clone(),
                attr_path: manifest_descriptor.pkg_path.clone(),
                derivation: None,
                version: manifest_descriptor.version.clone(),
                allow_pre_releases: manifest.options.semver.allow_pre_releases,
                allow_broken: manifest.options.allow.broken,
                // TODO: add support for insecure
                allow_insecure: None,
                allow_unfree: manifest.options.allow.unfree,
                allow_missing_builds: None,
                allowed_licenses: maybe_licenses.clone(),
                systems: vec![],
            };

            let group_name = manifest_descriptor
                .pkg_group
                .as_deref()
                .unwrap_or(DEFAULT_GROUP_NAME);

            let resolved_group =
                map.entry(group_name.to_string())
                    .or_insert_with(|| PackageGroup {
                        descriptors: Vec::new(),
                        name: group_name.to_string(),
                    });

            let systems = {
                let available_systems = manifest_systems.unwrap_or(&*DEFAULT_SYSTEMS_STR);

                let package_systems = manifest_descriptor.systems.as_deref();

                for system in package_systems.into_iter().flatten() {
                    if !available_systems.contains(system) {
                        return Err(ResolveError::SystemUnavailableInManifest {
                            install_id: install_id.clone(),
                            system: system.to_string(),
                            enabled_systems: available_systems
                                .iter()
                                .map(|s| s.to_string())
                                .collect(),
                        });
                    }
                }

                package_systems
                    .or(manifest_systems)
                    .unwrap_or(&*DEFAULT_SYSTEMS_STR)
                    .iter()
                    .sorted()
                    .map(|s| {
                        PackageSystem::from_str(s)
                            .map_err(|_| ResolveError::UnrecognizedSystem(s.to_string()))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };

            for system in systems {
                // If the package was just added to the manifest, it will be missing in the seed,
                // which is derived from the _previous_ lockfile.
                // In this case, the derivation will be None, and the package will be unconstrained.
                //
                // If the package was already locked, but the descriptor has changed in a way
                // that invalidates the existing resolution, the derivation will be None.
                //
                // If the package was locked from a flake installable before
                // it needs to be re-resolved with the catalog, so the derivation will be None.
                let locked_derivation = seed_locked_packages
                    .get(&(install_id, &system.to_string()))
                    .filter(|(descriptor, _)| {
                        !descriptor.invalidates_existing_resolution(&manifest_descriptor.into())
                    })
                    .and_then(|(_, locked_package)| locked_package.as_catalog_package_ref())
                    .map(|locked_package| locked_package.derivation.clone());

                let mut resolved_descriptor = resolved_descriptor_base.clone();

                resolved_descriptor.systems = vec![system];
                resolved_descriptor.derivation = locked_derivation;

                resolved_group.descriptors.push(resolved_descriptor);
            }
        }
        Ok(map.into_values())
    }

    /// Eliminate groups that are already fully locked
    /// by extracting them into a separate list of locked packages.
    ///
    /// This is used to avoid re-resolving packages that are already locked.
    fn split_fully_locked_groups(
        groups: impl IntoIterator<Item = PackageGroup>,
        seed_lockfile: Option<&Lockfile>,
    ) -> (Vec<LockedPackage>, Vec<PackageGroup>) {
        let seed_locked_packages = seed_lockfile.map_or_else(HashMap::new, Self::make_seed_mapping);

        let (already_locked_groups, groups_to_lock): (Vec<_>, Vec<_>) =
            groups.into_iter().partition(|group| {
                group
                    .descriptors
                    .iter()
                    .all(|descriptor| descriptor.derivation.is_some())
            });

        // convert already locked groups back to locked packages
        let already_locked_packages = already_locked_groups
            .iter()
            .flat_map(|group| &group.descriptors)
            .flat_map(|descriptor| {
                std::iter::repeat(&descriptor.install_id).zip(&descriptor.systems)
            })
            .filter_map(|(install_id, system)| {
                seed_locked_packages
                    .get(&(install_id, &system.to_string()))
                    .map(|(_, locked_package)| (*locked_package).to_owned())
            })
            .collect::<Vec<_>>();

        (already_locked_packages, groups_to_lock)
    }

    /// Convert resolution results into a list of locked packages
    ///
    /// * Flattens `Group(Page(PackageResolutionInfo+)+)` into `LockedPackageCatalog+`
    /// * Adds a `system` field to each locked package.
    /// * Converts [serde_json::Value] based `outputs` and `outputs_to_install` fields
    ///   into [`IndexMap<String, String>`] and [`Vec<String>`] respectively.
    ///
    /// TODO: handle results from multiple pages
    ///       currently there is no api to request packages from specific pages
    /// TODO: handle json value conversion earlier in the shim (or the upstream spec)
    fn locked_packages_from_resolution<'manifest>(
        manifest: &'manifest Manifest,
        groups: impl IntoIterator<Item = ResolvedPackageGroup> + 'manifest,
    ) -> Result<impl Iterator<Item = LockedPackageCatalog> + 'manifest, ResolveError> {
        let groups = groups.into_iter().collect::<Vec<_>>();
        let failed_group_indices = Self::detect_failed_resolutions(&groups);
        let failures = if failed_group_indices.is_empty() {
            tracing::debug!("no resolution failures detected");
            None
        } else {
            tracing::debug!("resolution failures detected");
            let failed_groups = failed_group_indices
                .iter()
                .map(|&i| groups[i].clone())
                .collect::<Vec<_>>();
            let failures = Self::collect_failures(&failed_groups, manifest)?;
            Some(failures)
        };
        if let Some(failures) = failures
            && !failures.is_empty()
        {
            tracing::debug!(n = failures.len(), "returning resolution failures");
            return Err(ResolveError::ResolutionFailed(ResolutionFailures(failures)));
        }
        let locked_pkg_iter = groups
            .into_iter()
            .flat_map(|group| {
                group
                    .page
                    .and_then(|p| p.packages.clone())
                    .map(|pkgs| pkgs.into_iter())
                    .ok_or(ResolveError::ResolutionFailed(
                        // This should be unreachable, otherwise we would have detected
                        // it as a failure
                        ResolutionFailure::FallbackMessage {
                            msg: "catalog page wasn't complete".into(),
                        }
                        .into(),
                    ))
            })
            .flatten()
            .filter_map(|resolved_pkg| {
                manifest
                    .catalog_pkg_descriptor_with_id(&resolved_pkg.install_id)
                    .map(|descriptor| LockedPackageCatalog::from_parts(resolved_pkg, descriptor))
            });
        Ok(locked_pkg_iter)
    }

    /// Constructs [ResolutionFailure]s from the failed groups
    fn collect_failures(
        failed_groups: &[ResolvedPackageGroup],
        manifest: &Manifest,
    ) -> Result<Vec<ResolutionFailure>, ResolveError> {
        let mut failures = Vec::new();
        for group in failed_groups {
            tracing::debug!(
                name = group.name,
                "collecting failures from unresolved group"
            );
            for res_msg in group.msgs.iter() {
                tracing::debug!(
                    level = res_msg.level().to_string(),
                    msg = res_msg.msg(),
                    "handling resolution message"
                );
                // If it's not an error, skip this message
                if res_msg.level() != MessageLevel::Error {
                    continue;
                }
                let failure = match res_msg {
                    catalog::ResolutionMessage::General(inner) => {
                        tracing::debug!(kind = "general");
                        ResolutionFailure::FallbackMessage {
                            msg: inner.msg.clone(),
                        }
                    },
                    catalog::ResolutionMessage::AttrPathNotFoundNotInCatalog(inner) => {
                        tracing::debug!(kind = "attr_path_not_found.not_in_catalog",);
                        ResolutionFailure::PackageNotFound(inner.clone())
                    },
                    catalog::ResolutionMessage::AttrPathNotFoundNotFoundForAllSystems(inner) => {
                        tracing::debug!(kind = "attr_path_not_found.not_found_for_all_systems",);
                        ResolutionFailure::PackageUnavailableOnSomeSystems {
                            catalog_message: inner.clone(),
                            invalid_systems: Self::determine_invalid_systems(inner, manifest)?,
                        }
                    },
                    catalog::ResolutionMessage::AttrPathNotFoundSystemsNotOnSamePage(inner) => {
                        tracing::debug!(kind = "attr_path_not_found.systems_not_on_same_page");
                        ResolutionFailure::SystemsNotOnSamePage(inner.clone())
                    },
                    catalog::ResolutionMessage::ConstraintsTooTight(inner) => {
                        tracing::debug!(kind = "constraints_too_tight",);
                        ResolutionFailure::ConstraintsTooTight {
                            catalog_message: inner.clone(),
                            group: group.name.clone(),
                        }
                    },
                    catalog::ResolutionMessage::Unknown(inner) => {
                        tracing::debug!(
                            kind = "unknown",
                            msg_type = inner.msg_type,
                            context = serde_json::to_string(&inner.context).unwrap(),
                            "handling unknown resolution message"
                        );
                        ResolutionFailure::UnknownServiceMessage(inner.clone())
                    },
                };
                failures.push(failure);
            }
        }
        Ok(failures)
    }

    /// Determines which systems a package was requested on that it is not
    /// available for
    fn determine_invalid_systems(
        r_msg: &MsgAttrPathNotFoundNotFoundForAllSystems,
        manifest: &Manifest,
    ) -> Result<Vec<System>, ResolveError> {
        let default_systems = HashSet::<_>::from_iter(DEFAULT_SYSTEMS_STR.iter());
        let valid_systems = HashSet::<_>::from_iter(&r_msg.valid_systems);
        let manifest_systems = manifest
            .options
            .systems
            .as_ref()
            .map(HashSet::<_>::from_iter)
            .unwrap_or(default_systems);
        let pkg_descriptor = manifest
            .catalog_pkg_descriptor_with_id(&r_msg.install_id)
            .ok_or(ResolveError::InstallIdNotInManifest(
                r_msg.install_id.clone(),
            ))?;
        let pkg_systems = pkg_descriptor.systems.as_ref().map(HashSet::from_iter);
        let requested_systems = pkg_systems.unwrap_or(manifest_systems);
        let difference = &requested_systems - &valid_systems;
        Ok(Vec::from_iter(difference.into_iter().cloned()))
    }

    /// Detects whether any groups failed to resolve
    fn detect_failed_resolutions(groups: &[ResolvedPackageGroup]) -> Vec<usize> {
        groups
            .iter()
            .enumerate()
            .filter_map(|(idx, group)| {
                if group.page.is_none() {
                    tracing::debug!(name = group.name, "detected unresolved group");
                    Some(idx)
                } else if group.page.as_ref().is_some_and(|p| !p.complete) {
                    tracing::debug!(name = group.name, "detected incomplete page");
                    Some(idx)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    }

    /// Collect flake installable descriptors from the manifest and create a list of
    /// [FlakeInstallableToLock] to be resolved.
    /// Each descriptor is resolved once per system supported by the manifest,
    /// or other if not specified, for each system in [DEFAULT_SYSTEMS_STR].
    ///
    /// Unlike catalog packages, [FlakeInstallableToLock] are not affected by a seed lockfile.
    /// Already locked flake installables are split from the list in the second step using
    /// [Self::split_locked_flake_installables], based on the descriptor alone,
    /// no additional "marking" is needed.
    fn collect_flake_installables(
        manifest: &Manifest,
    ) -> impl Iterator<Item = FlakeInstallableToLock> + '_ {
        manifest
            .install
            .inner()
            .iter()
            .filter_map(|(install_id, descriptor)| {
                descriptor
                    .as_flake_descriptor_ref()
                    .map(|d| (install_id, d))
            })
            .flat_map(|(iid, d)| {
                let systems = if let Some(ref d_systems) = d.systems {
                    d_systems.as_slice()
                } else {
                    manifest
                        .options
                        .systems
                        .as_deref()
                        .unwrap_or(&*DEFAULT_SYSTEMS_STR)
                };
                systems.iter().map(move |s| FlakeInstallableToLock {
                    install_id: iid.clone(),
                    descriptor: d.clone(),
                    system: s.clone(),
                })
            })
    }

    /// Split a list of flake installables into already Locked packages ([LockedPackage])
    /// and yet to lock [FlakeInstallableToLock].
    ///
    /// This is equivalent to [Self::split_fully_locked_groups] but for flake installables.
    /// where `installables` are the flake installables found in a lockfile,
    /// with [Self::collect_flake_installables].
    fn split_locked_flake_installables(
        installables: impl IntoIterator<Item = FlakeInstallableToLock>,
        seed_lockfile: Option<&Lockfile>,
    ) -> (Vec<LockedPackage>, Vec<FlakeInstallableToLock>) {
        // todo: consider computing once and passing a reference to the consumer functions.
        //       we now compute this 3 times during a single lock operation
        let seed_locked_packages = seed_lockfile.map_or_else(HashMap::new, Self::make_seed_mapping);

        let by_id = installables.into_iter().chunk_by(|i| i.install_id.clone());

        let (already_locked, to_lock): (Vec<Vec<LockedPackage>>, Vec<Vec<FlakeInstallableToLock>>) =
            by_id.into_iter().partition_map(|(_, group)| {
                let unlocked = group.collect::<Vec<_>>();
                let mut locked = Vec::new();

                for installable in unlocked.iter() {
                    let Some((locked_descriptor, in_lockfile @ LockedPackage::Flake(_))) =
                        seed_locked_packages
                            .get(&(installable.install_id.as_str(), &installable.system))
                    else {
                        return Either::Right(unlocked);
                    };

                    if ManifestPackageDescriptor::from(installable.descriptor.clone())
                        .invalidates_existing_resolution(locked_descriptor)
                    {
                        return Either::Right(unlocked);
                    }

                    locked.push((*in_lockfile).to_owned());
                }
                Either::Left(locked)
            });

        let already_locked = already_locked.into_iter().flatten().collect();
        let to_lock = to_lock.into_iter().flatten().collect();

        (already_locked, to_lock)
    }

    /// Lock a set of flake installables and return the locked packages.
    /// Errors are collected into [ResolutionFailures] and returned as a single error.
    ///
    /// This is the eequivalent to
    /// [catalog::ClientTrait::resolve] and passing the result to [Self::locked_packages_from_resolution]
    /// in the context of flake installables.
    /// At this point flake installables are resolved sequentially.
    /// In further iterations we may want to resolve them in parallel,
    /// either here, through a method of [InstallableLocker],
    /// or the underlying `lock-flake-installable` primop itself.
    ///
    /// Todo: [ResolutionFailures] may be caught downstream and used to provide suggestions.
    ///       Those suggestions are invalid for the flake installables case.
    fn lock_flake_installables<'locking>(
        locking: &'locking impl InstallableLocker,
        installables: impl IntoIterator<Item = FlakeInstallableToLock> + 'locking,
    ) -> Result<impl Iterator<Item = LockedPackageFlake> + 'locking, ResolveError> {
        let mut ok = Vec::new();
        for installable in installables.into_iter() {
            match locking
                .lock_flake_installable(&installable.system, &installable.descriptor)
                .map(|locked_installable| {
                    LockedPackageFlake::from_parts(installable.install_id, locked_installable)
                }) {
                Ok(locked) => ok.push(locked),
                Err(e) => {
                    if let FlakeInstallableError::NixError(_) = e {
                        return Err(ResolveError::LockFlakeNixError(e));
                    }
                    let failure = ResolutionFailure::FallbackMessage { msg: e.to_string() };
                    return Err(ResolveError::ResolutionFailed(ResolutionFailures(vec![
                        failure,
                    ])));
                },
            }
        }
        Ok(ok.into_iter())
    }

    /// Collect store paths from the manifest and create a list of [LockedPackageStorePath].
    /// Since store paths are locked by definition,
    /// collection can directly map the discriptor to a locked package.
    fn collect_store_paths(manifest: &Manifest) -> Vec<LockedPackageStorePath> {
        manifest
            .install
            .inner()
            .iter()
            .filter_map(|(install_id, descriptor)| {
                descriptor
                    .as_store_path_descriptor_ref()
                    .map(|d| (install_id, d))
            })
            .flat_map(|(install_id, descriptor)| {
                let systems = if let Some(ref d_systems) = descriptor.systems {
                    d_systems.as_slice()
                } else {
                    manifest
                        .options
                        .systems
                        .as_deref()
                        .unwrap_or(&*DEFAULT_SYSTEMS_STR)
                };

                systems.iter().map(move |system| LockedPackageStorePath {
                    install_id: install_id.clone(),
                    store_path: descriptor.store_path.clone(),
                    system: system.clone(),
                    priority: descriptor.priority.unwrap_or(DEFAULT_PRIORITY),
                })
            })
            .collect()
    }

    /// Filter out packages from the locked manifest by install_id or group
    /// If groups_or_iids is empty, all packages are unlocked.
    ///
    /// This is used to create a seed lockfile to upgrade a subset of packages,
    /// as packages that are not in the seed lockfile will be re-resolved unconstrained.
    pub(crate) fn unlock_packages_by_group_or_iid(&mut self, groups_or_iids: &[&str]) -> &mut Self {
        if groups_or_iids.is_empty() {
            self.packages = Vec::new();
        } else {
            self.packages = std::mem::take(&mut self.packages)
                .into_iter()
                .filter(|package| {
                    if groups_or_iids.contains(&package.install_id()) {
                        return false;
                    }

                    if let Some(catalog_package) = package.as_catalog_package_ref() {
                        return !groups_or_iids.contains(&catalog_package.group.as_str());
                    }

                    true
                })
                .collect();
        }
        self
    }

    /// Eliminate groups that are already fully locked
    /// by extracting them into a separate list of locked packages.
    ///
    /// This is used to avoid re-resolving packages that are already locked.
    fn split_fully_locked_groups(
        groups: impl IntoIterator<Item = PackageGroup>,
        seed_lockfile: Option<&Lockfile>,
    ) -> (Vec<LockedPackage>, Vec<PackageGroup>) {
        let seed_locked_packages = seed_lockfile.map_or_else(HashMap::new, Self::make_seed_mapping);

        let (already_locked_groups, groups_to_lock): (Vec<_>, Vec<_>) =
            groups.into_iter().partition(|group| {
                group
                    .descriptors
                    .iter()
                    .all(|descriptor| descriptor.derivation.is_some())
            });

        // convert already locked groups back to locked packages
        let already_locked_packages = already_locked_groups
            .iter()
            .flat_map(|group| &group.descriptors)
            .flat_map(|descriptor| {
                std::iter::repeat(&descriptor.install_id).zip(&descriptor.systems)
            })
            .filter_map(|(install_id, system)| {
                seed_locked_packages
                    .get(&(install_id, &system.to_string()))
                    .map(|(_, locked_package)| (*locked_package).to_owned())
            })
            .collect::<Vec<_>>();

        (already_locked_packages, groups_to_lock)
    }

    /// Convert resolution results into a list of locked packages
    ///
    /// * Flattens `Group(Page(PackageResolutionInfo+)+)` into `LockedPackageCatalog+`
    /// * Adds a `system` field to each locked package.
    /// * Converts [serde_json::Value] based `outputs` and `outputs_to_install` fields
    ///   into [`IndexMap<String, String>`] and [`Vec<String>`] respectively.
    ///
    /// TODO: handle results from multiple pages
    ///       currently there is no api to request packages from specific pages
    /// TODO: handle json value conversion earlier in the shim (or the upstream spec)
    fn locked_packages_from_resolution<'manifest>(
        manifest: &'manifest Manifest,
        groups: impl IntoIterator<Item = ResolvedPackageGroup> + 'manifest,
    ) -> Result<impl Iterator<Item = LockedPackageCatalog> + 'manifest, ResolveError> {
        let groups = groups.into_iter().collect::<Vec<_>>();
        let failed_group_indices = Self::detect_failed_resolutions(&groups);
        let failures = if failed_group_indices.is_empty() {
            tracing::debug!("no resolution failures detected");
            None
        } else {
            tracing::debug!("resolution failures detected");
            let failed_groups = failed_group_indices
                .iter()
                .map(|&i| groups[i].clone())
                .collect::<Vec<_>>();
            let failures = Self::collect_failures(&failed_groups, manifest)?;
            Some(failures)
        };
        if let Some(failures) = failures
            && !failures.is_empty()
        {
            tracing::debug!(n = failures.len(), "returning resolution failures");
            return Err(ResolveError::ResolutionFailed(ResolutionFailures(failures)));
        }
        let locked_pkg_iter = groups
            .into_iter()
            .flat_map(|group| {
                group
                    .page
                    .and_then(|p| p.packages.clone())
                    .map(|pkgs| pkgs.into_iter())
                    .ok_or(ResolveError::ResolutionFailed(
                        // This should be unreachable, otherwise we would have detected
                        // it as a failure
                        ResolutionFailure::FallbackMessage {
                            msg: "catalog page wasn't complete".into(),
                        }
                        .into(),
                    ))
            })
            .flatten()
            .filter_map(|resolved_pkg| {
                manifest
                    .catalog_pkg_descriptor_with_id(&resolved_pkg.install_id)
                    .map(|descriptor| LockedPackageCatalog::from_parts(resolved_pkg, descriptor))
            });
        Ok(locked_pkg_iter)
    }

    /// Determines which systems a package was requested on that it is not
    /// available for
    fn determine_invalid_systems(
        r_msg: &MsgAttrPathNotFoundNotFoundForAllSystems,
        manifest: &Manifest,
    ) -> Result<Vec<System>, ResolveError> {
        let default_systems = HashSet::<_>::from_iter(DEFAULT_SYSTEMS_STR.iter());
        let valid_systems = HashSet::<_>::from_iter(&r_msg.valid_systems);
        let manifest_systems = manifest
            .options
            .systems
            .as_ref()
            .map(HashSet::<_>::from_iter)
            .unwrap_or(default_systems);
        let pkg_descriptor = manifest
            .catalog_pkg_descriptor_with_id(&r_msg.install_id)
            .ok_or(ResolveError::InstallIdNotInManifest(
                r_msg.install_id.clone(),
            ))?;
        let pkg_systems = pkg_descriptor.systems.as_ref().map(HashSet::from_iter);
        let requested_systems = pkg_systems.unwrap_or(manifest_systems);
        let difference = &requested_systems - &valid_systems;
        Ok(Vec::from_iter(difference.into_iter().cloned()))
    }

    /// Detects whether any groups failed to resolve
    fn detect_failed_resolutions(groups: &[ResolvedPackageGroup]) -> Vec<usize> {
        groups
            .iter()
            .enumerate()
            .filter_map(|(idx, group)| {
                if group.page.is_none() {
                    tracing::debug!(name = group.name, "detected unresolved group");
                    Some(idx)
                } else if group.page.as_ref().is_some_and(|p| !p.complete) {
                    tracing::debug!(name = group.name, "detected incomplete page");
                    Some(idx)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    }

    /// Collect flake installable descriptors from the manifest and create a list of
    /// [FlakeInstallableToLock] to be resolved.
    /// Each descriptor is resolved once per system supported by the manifest,
    /// or other if not specified, for each system in [DEFAULT_SYSTEMS_STR].
    ///
    /// Unlike catalog packages, [FlakeInstallableToLock] are not affected by a seed lockfile.
    /// Already locked flake installables are split from the list in the second step using
    /// [Self::split_locked_flake_installables], based on the descriptor alone,
    /// no additional "marking" is needed.
    fn collect_flake_installables(
        manifest: &Manifest,
    ) -> impl Iterator<Item = FlakeInstallableToLock> + '_ {
        manifest
            .install
            .inner()
            .iter()
            .filter_map(|(install_id, descriptor)| {
                descriptor
                    .as_flake_descriptor_ref()
                    .map(|d| (install_id, d))
            })
            .flat_map(|(iid, d)| {
                let systems = if let Some(ref d_systems) = d.systems {
                    d_systems.as_slice()
                } else {
                    manifest
                        .options
                        .systems
                        .as_deref()
                        .unwrap_or(&*DEFAULT_SYSTEMS_STR)
                };
                systems.iter().map(move |s| FlakeInstallableToLock {
                    install_id: iid.clone(),
                    descriptor: d.clone(),
                    system: s.clone(),
                })
            })
    }

    /// Split a list of flake installables into already Locked packages ([LockedPackage])
    /// and yet to lock [FlakeInstallableToLock].
    ///
    /// This is equivalent to [Self::split_fully_locked_groups] but for flake installables.
    /// where `installables` are the flake installables found in a lockfile,
    /// with [Self::collect_flake_installables].
    fn split_locked_flake_installables(
        installables: impl IntoIterator<Item = FlakeInstallableToLock>,
        seed_lockfile: Option<&Lockfile>,
    ) -> (Vec<LockedPackage>, Vec<FlakeInstallableToLock>) {
        // todo: consider computing once and passing a reference to the consumer functions.
        //       we now compute this 3 times during a single lock operation
        let seed_locked_packages = seed_lockfile.map_or_else(HashMap::new, Self::make_seed_mapping);

        let by_id = installables.into_iter().chunk_by(|i| i.install_id.clone());

        let (already_locked, to_lock): (Vec<Vec<LockedPackage>>, Vec<Vec<FlakeInstallableToLock>>) =
            by_id.into_iter().partition_map(|(_, group)| {
                let unlocked = group.collect::<Vec<_>>();
                let mut locked = Vec::new();

                for installable in unlocked.iter() {
                    let Some((locked_descriptor, in_lockfile @ LockedPackage::Flake(_))) =
                        seed_locked_packages
                            .get(&(installable.install_id.as_str(), &installable.system))
                    else {
                        return Either::Right(unlocked);
                    };

                    if ManifestPackageDescriptor::from(installable.descriptor.clone())
                        .invalidates_existing_resolution(locked_descriptor)
                    {
                        return Either::Right(unlocked);
                    }

                    locked.push((*in_lockfile).to_owned());
                }
                Either::Left(locked)
            });

        let already_locked = already_locked.into_iter().flatten().collect();
        let to_lock = to_lock.into_iter().flatten().collect();

        (already_locked, to_lock)
    }

    /// Lock a set of flake installables and return the locked packages.
    /// Errors are collected into [ResolutionFailures] and returned as a single error.
    ///
    /// This is the eequivalent to
    /// [catalog::ClientTrait::resolve] and passing the result to [Self::locked_packages_from_resolution]
    /// in the context of flake installables.
    /// At this point flake installables are resolved sequentially.
    /// In further iterations we may want to resolve them in parallel,
    /// either here, through a method of [InstallableLocker],
    /// or the underlying `lock-flake-installable` primop itself.
    ///
    /// Todo: [ResolutionFailures] may be caught downstream and used to provide suggestions.
    ///       Those suggestions are invalid for the flake installables case.
    fn lock_flake_installables<'locking>(
        locking: &'locking impl InstallableLocker,
        installables: impl IntoIterator<Item = FlakeInstallableToLock> + 'locking,
    ) -> Result<impl Iterator<Item = LockedPackageFlake> + 'locking, ResolveError> {
        let mut ok = Vec::new();
        for installable in installables.into_iter() {
            match locking
                .lock_flake_installable(&installable.system, &installable.descriptor)
                .map(|locked_installable| {
                    LockedPackageFlake::from_parts(installable.install_id, locked_installable)
                }) {
                Ok(locked) => ok.push(locked),
                Err(e) => {
                    if let FlakeInstallableError::NixError(_) = e {
                        return Err(ResolveError::LockFlakeNixError(e));
                    }
                    let failure = ResolutionFailure::FallbackMessage { msg: e.to_string() };
                    return Err(ResolveError::ResolutionFailed(ResolutionFailures(vec![
                        failure,
                    ])));
                },
            }
        }
        Ok(ok.into_iter())
    }

    /// Collect store paths from the manifest and create a list of [LockedPackageStorePath].
    /// Since store paths are locked by definition,
    /// collection can directly map the discriptor to a locked package.
    fn collect_store_paths(manifest: &Manifest) -> Vec<LockedPackageStorePath> {
        manifest
            .install
            .inner()
            .iter()
            .filter_map(|(install_id, descriptor)| {
                descriptor
                    .as_store_path_descriptor_ref()
                    .map(|d| (install_id, d))
            })
            .flat_map(|(install_id, descriptor)| {
                let systems = if let Some(ref d_systems) = descriptor.systems {
                    d_systems.as_slice()
                } else {
                    manifest
                        .options
                        .systems
                        .as_deref()
                        .unwrap_or(&*DEFAULT_SYSTEMS_STR)
                };

                systems.iter().map(move |system| LockedPackageStorePath {
                    install_id: install_id.clone(),
                    store_path: descriptor.store_path.clone(),
                    system: system.clone(),
                    priority: descriptor.priority.unwrap_or(DEFAULT_PRIORITY),
                })
            })
            .collect()
    }

    /// Filter out packages from the locked manifest by install_id or group
    /// If groups_or_iids is empty, all packages are unlocked.
    ///
    /// This is used to create a seed lockfile to upgrade a subset of packages,
    /// as packages that are not in the seed lockfile will be re-resolved unconstrained.
    pub(crate) fn unlock_packages_by_group_or_iid(&mut self, groups_or_iids: &[&str]) -> &mut Self {
        if groups_or_iids.is_empty() {
            self.packages = Vec::new();
        } else {
            self.packages = std::mem::take(&mut self.packages)
                .into_iter()
                .filter(|package| {
                    if groups_or_iids.contains(&package.install_id()) {
                        return false;
                    }

                    if let Some(catalog_package) = package.as_catalog_package_ref() {
                        return !groups_or_iids.contains(&catalog_package.group.as_str());
                    }

                    true
                })
                .collect();
        }
        self
    }
}

/// All the resolution failures for a single resolution request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionFailures(pub Vec<ResolutionFailure>);

impl FromIterator<ResolutionFailure> for ResolutionFailures {
    fn from_iter<T: IntoIterator<Item = ResolutionFailure>>(iter: T) -> Self {
        ResolutionFailures(iter.into_iter().collect())
    }
}

/// Data relevant for formatting a resolution failure
///
/// This may wrap messages returned from the catalog with additional information
/// extracted from the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ResolutionFailure {
    PackageNotFound(MsgAttrPathNotFoundNotInCatalog),
    PackageUnavailableOnSomeSystems {
        catalog_message: MsgAttrPathNotFoundNotFoundForAllSystems,
        invalid_systems: Vec<String>,
    },
    SystemsNotOnSamePage(MsgAttrPathNotFoundSystemsNotOnSamePage),
    ConstraintsTooTight {
        catalog_message: MsgConstraintsTooTight,
        group: String,
    },
    UnknownServiceMessage(MsgUnknown),
    FallbackMessage {
        msg: String,
    },
}

// Convenience for when you just have a single message
impl From<ResolutionFailure> for ResolutionFailures {
    fn from(value: ResolutionFailure) -> Self {
        ResolutionFailures::from_iter([value])
    }
}

impl Display for ResolutionFailures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let formatted = if self.0.len() > 1 {
            format_multiple_resolution_failures(&self.0)
        } else {
            format_single_resolution_failure(&self.0[0], false)
        };
        write!(f, "{formatted}")
    }
}

/// Formats a single resolution failure in a nice way
fn format_single_resolution_failure(failure: &ResolutionFailure, is_one_of_many: bool) -> String {
    match failure {
        ResolutionFailure::PackageNotFound(MsgAttrPathNotFoundNotInCatalog {
            attr_path, ..
        }) => {
            // Note: for `flox install`, this variant will be formatted with the
            // "didyoumean" mechanism.
            format!("could not find package '{attr_path}'.")
        },
        ResolutionFailure::PackageUnavailableOnSomeSystems {
            catalog_message:
                MsgAttrPathNotFoundNotFoundForAllSystems {
                    attr_path,
                    valid_systems,
                    ..
                },
            invalid_systems,
            ..
        } => {
            let extra_indent = if is_one_of_many { 2 } else { 0 };
            let indented_invalid = invalid_systems
                .iter()
                .sorted()
                .map(|s| indent_all_by(4, format!("- {s}")))
                .join("\n");
            let indented_valid = valid_systems
                .iter()
                .sorted()
                .map(|s| indent_all_by(4, format!("- {s}")))
                .join("\n");
            let listed = [
                format!("package '{attr_path}' not available for"),
                indented_invalid,
                indent_all_by(2, "but it is available for"),
                indented_valid,
            ]
            .join("\n");
            let with_doc_link = formatdoc! {"
            {listed}

            For more on managing system-specific packages, visit the documentation:
            https://flox.dev/docs/tutorials/multi-arch-environments/#handling-unsupported-packages"};
            indent_by(extra_indent, with_doc_link)
        },
        ResolutionFailure::ConstraintsTooTight { group, .. } => {
            let extra_indent = if is_one_of_many { 2 } else { 3 };
            let base_msg = format!("constraints for group '{group}' are too tight");
            let msg = formatdoc! {"
            {base_msg}

            Use 'flox edit' to adjust version constraints in the [install] section,
            or isolate dependencies in a new group with '<pkg>.pkg-group = \"newgroup\"'"};
            indent_by(extra_indent, msg)
        },
        ResolutionFailure::SystemsNotOnSamePage(MsgAttrPathNotFoundSystemsNotOnSamePage {
            msg,
            ..
        })
        | ResolutionFailure::UnknownServiceMessage(MsgUnknown { msg, .. })
        | ResolutionFailure::FallbackMessage { msg } => {
            if is_one_of_many {
                indent_by(2, msg.to_string())
            } else {
                format!("\n{}", msg)
            }
        },
    }
}

/// Formats several resolution messages in a more legible way than just one per line
fn format_multiple_resolution_failures(failures: &[ResolutionFailure]) -> String {
    let msgs = failures
        .iter()
        .map(|f| format!("- {}", format_single_resolution_failure(f, true)))
        .collect::<Vec<_>>()
        .join("\n");
    format!("multiple resolution failures:\n{msgs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::flake_installable_locker::InstallableLocker;

    struct PanickingLocker;
    impl InstallableLocker for PanickingLocker {
        fn lock_flake_installable(
            &self,
            _: impl AsRef<str>,
            _: &PackageDescriptorFlake,
        ) -> Result<LockedInstallable, FlakeInstallableError> {
            panic!("this flake locker always panics")
        }
    }

    #[test]
    fn make_params_smoke() {
        let manifest = &*TEST_TYPED_MANIFEST;

        let params = Lockfile::collect_package_groups(manifest, None)
            .unwrap()
            .collect::<Vec<_>>();
        assert_eq!(&params, &*TEST_RESOLUTION_PARAMS);
    }

    /// When `options.systems` defines multiple systems,
    /// request groups for each system separately.
    #[test]
    fn make_params_multiple_systems() {
        let manifest_str = indoc! {r#"
            version = 1

            [install]
            vim.pkg-path = "vim"
            emacs.pkg-path = "emacs"

            [options]
            systems = ["aarch64-darwin", "x86_64-linux"]
        "#};
        let manifest = toml::from_str(manifest_str).unwrap();

        let expected_params = vec![PackageGroup {
            name: DEFAULT_GROUP_NAME.to_string(),
            descriptors: vec![
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "emacs".to_string(),
                    derivation: None,
                    install_id: "emacs".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "emacs".to_string(),
                    derivation: None,
                    install_id: "emacs".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::X8664Linux],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::X8664Linux],
                },
            ],
        }];

        let actual_params = Lockfile::collect_package_groups(&manifest, None)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(actual_params, expected_params);
    }

    /// When `options.systems` defines multiple systems,
    /// request groups for each system separately.
    /// If a package specifies systems, use those instead.
    #[test]
    fn make_params_limit_systems() {
        let manifest_str = indoc! {r#"
            version = 1

            [install]
            vim.pkg-path = "vim"
            emacs.pkg-path = "emacs"
            emacs.systems = ["aarch64-darwin" ]

            [options]
            systems = ["aarch64-darwin", "x86_64-linux"]
        "#};
        let manifest = toml::from_str(manifest_str).unwrap();

        let expected_params = vec![PackageGroup {
            name: DEFAULT_GROUP_NAME.to_string(),
            descriptors: vec![
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "emacs".to_string(),
                    install_id: "emacs".to_string(),
                    derivation: None,
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::X8664Linux],
                },
            ],
        }];

        let actual_params = Lockfile::collect_package_groups(&manifest, None)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(actual_params, expected_params);
    }
}
