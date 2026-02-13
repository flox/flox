//! The following trait and macro need explanation.
//!
//! When we deserialize a lockfile, we then sometimes query the manifests contained
//! within it (e.g. for composition). The different manifest schemas are so
//! painfully similar, and yet because they're different types we can't just
//! write a single method that does e.g. `mymanifest.install.inner().some_operation()`.
//!
//! The options are:
//! - Require a lockfile be present for every single operation so that we can
//!   migrate in all cases and therefore be able to rely on a single manifest
//!   schema. Painful. Bad. Not good.
//! - Create some kind of generic interface that is able to use the fact that
//!   the structure is extremely similar between manifest schemas, while papering
//!   over the fact that they're actually different types.
//!
//! We're opting to do the latter option via this trait and a macro. The macro
//! takes a module path as the first argument, and the manifest type name as the
//! second argument. Then it relies on the fact that (so far) all of the manifest
//! modules have a consistent layout:
//! - <schema module>
//!   - <manifest type>
//!   - `package_descriptor` (module)
//!     - `ManifestPackageDescriptor` (type)
//!     - `PackageDescriptorCatalog` (type)
//!     - `PackageDescriptorFlake` (type)
//!
//! If we know the module path and the manifest type, we can interpolate them
//! into the trait implementation via the macro. Yes it's gross, but it's better
//! than the alternative.

mod migrated;
use std::collections::BTreeMap;

#[allow(unused_imports)]
pub use migrated::*;

use crate::ManifestError;

/// An interface for looking up packages in a manifest.
pub trait PackageLookup {
    type PkgDescriptor;
    type CatalogDescriptor;
    type FlakeDescriptor;

    /// Returns a map of package group name to a collection of package descriptors in that group.
    /// The collection of packages in the package group are stored in a map indexed by their
    /// install IDs.
    fn catalog_pkgs_by_group(&self) -> BTreeMap<String, BTreeMap<String, Self::CatalogDescriptor>>;

    /// Locates the package descriptor with the provided install ID.
    fn pkg_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<Self::PkgDescriptor>;

    /// Locates the catalog package descriptor with the provided install ID,
    /// returning `None` if there was a package descriptor of another kind
    /// with the desired install ID.
    fn catalog_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<Self::CatalogDescriptor>;

    /// Locates the flake package descriptor with the provided install ID,
    /// returning `None` if there was a package descriptor of another kind
    /// with the desired install ID.
    fn flake_pkg_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<Self::PkgDescriptor>;

    /// Returns a sequence of install IDs and associated package descriptors
    /// that are in the `toplevel` group of the manifest.
    fn pkg_descriptors_in_toplevel_group(&self) -> Vec<(String, Self::PkgDescriptor)>;

    /// Returns a sequence of install IDs and associated package descriptors
    /// that are in a package group with the provided name.
    fn pkg_descriptors_in_named_group(
        &self,
        name: impl AsRef<str>,
    ) -> Vec<(String, Self::PkgDescriptor)>;

    /// Returns `true` if an install ID or package group is found with the
    /// provided name.
    fn pkg_or_group_found_in_manifest(&self, name: impl AsRef<str>) -> bool;

    /// Returns `true` if the package belongs to a non-empty package group
    /// other than `toplevel`.
    fn pkg_belongs_to_non_empty_named_group(
        &self,
        pkg: impl AsRef<str>,
    ) -> Result<Option<String>, ManifestError>;

    /// Returns `true` if the `toplevel` package group is non-empty and contains
    /// the provided package name.
    fn pkg_belongs_to_non_empty_toplevel_group(
        &self,
        pkg: impl AsRef<str>,
    ) -> Result<bool, ManifestError>;

    /// Who knows what this does
    fn get_install_ids(&self, packages: Vec<String>) -> Result<Vec<String>, ManifestError>;
}

#[macro_export]
macro_rules! impl_pkg_lookup {
    ($manifest_module:path, $manifest:ty) => {
        // This `use` is necessary because you can't interpolate on either side
        // of a `path` fragment (you'll get an error). So, in order to be able
        // to refer to things inside the schema module we need to create an
        // alias (`concrete`) that we _can_ put `::` next to without the
        // compiler complaining.
        use $manifest_module as concrete;
        impl $crate::interfaces::PackageLookup for $manifest {
            type CatalogDescriptor = concrete::package_descriptor::PackageDescriptorCatalog;
            type FlakeDescriptor = concrete::package_descriptor::PackageDescriptorFlake;
            type PkgDescriptor = concrete::package_descriptor::ManifestPackageDescriptor;

            /// Returns a map of package group name to a collection of package descriptors in that group.
            /// The collection of packages in the package group are stored in a map indexed by their
            /// install IDs.
            fn catalog_pkgs_by_group(
                &self,
            ) -> BTreeMap<String, BTreeMap<String, Self::CatalogDescriptor>> {
                let mut groups = BTreeMap::new();
                for (id, descriptor) in self.install.inner().iter() {
                    if let Some(catalog_descriptor) = descriptor.as_catalog_descriptor_ref() {
                        let group_name = catalog_descriptor
                            .pkg_group
                            .clone()
                            .unwrap_or($crate::parsed::common::DEFAULT_GROUP_NAME.to_string());
                        let group_map: &mut BTreeMap<String, Self::CatalogDescriptor> =
                            groups.entry(group_name).or_default();
                        group_map.insert(id.clone(), catalog_descriptor.clone());
                    }
                }
                groups
            }

            /// Get the package descriptor with the specified install_id.
            fn pkg_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<Self::PkgDescriptor> {
                self.install.inner().get(id.as_ref()).cloned()
            }

            /// Get the package descriptor with the specified install_id.
            fn catalog_descriptor_with_id(
                &self,
                id: impl AsRef<str>,
            ) -> Option<Self::CatalogDescriptor> {
                self.install
                    .0
                    .get(id.as_ref())
                    .and_then(Self::PkgDescriptor::as_catalog_descriptor_ref)
                    .cloned()
            }

            /// Get the package descriptor with the specified install_id.
            fn flake_pkg_descriptor_with_id(
                &self,
                id: impl AsRef<str>,
            ) -> Option<ManifestPackageDescriptor> {
                self.install.0.get(id.as_ref()).cloned()
            }

            /// Get the package descriptors in the "toplevel" group.
            fn pkg_descriptors_in_toplevel_group(
                &self,
            ) -> Vec<(String, ManifestPackageDescriptor)> {
                self.install
                    .inner()
                    .iter()
                    .filter(|(_, desc)| {
                        let ManifestPackageDescriptor::Catalog(Self::CatalogDescriptor {
                            pkg_group,
                            ..
                        }) = desc
                        else {
                            return false;
                        };

                        pkg_group.is_none()
                    })
                    .map(|(id, desc)| (id.clone(), desc.clone()))
                    .collect::<Vec<_>>()
            }

            /// Get the package descriptors in a named group.
            fn pkg_descriptors_in_named_group(
                &self,
                name: impl AsRef<str>,
            ) -> Vec<(String, ManifestPackageDescriptor)> {
                self.install
                    .inner()
                    .iter()
                    .filter(|(_, desc)| {
                        let ManifestPackageDescriptor::Catalog(Self::CatalogDescriptor {
                            pkg_group,
                            ..
                        }) = desc
                        else {
                            return false;
                        };

                        pkg_group
                            .as_ref()
                            .is_some_and(|n| n.as_str() == name.as_ref())
                    })
                    .map(|(id, desc)| (id.clone(), desc.clone()))
                    .collect::<Vec<_>>()
            }

            /// Check whether the specified name is either an install_id or group name.
            fn pkg_or_group_found_in_manifest(&self, name: impl AsRef<str>) -> bool {
                self.install.inner().iter().any(|(id, desc)| {
                    let group = if let concrete::ManifestPackageDescriptor::Catalog(catalog) = desc
                    {
                        catalog.pkg_group.as_deref()
                    } else {
                        None
                    };

                    let search_term = name.as_ref();

                    (search_term == id.as_str()) || (Some(search_term) == group)
                })
            }

            /// Check whether the specified package belongs to a named group
            /// with additional packages.
            fn pkg_belongs_to_non_empty_named_group(
                &self,
                pkg: impl AsRef<str>,
            ) -> Result<Option<String>, ManifestError> {
                let descriptors = self.install.inner();
                let pkg = pkg.as_ref();
                let descriptor = descriptors
                    .get(pkg)
                    .ok_or(ManifestError::PkgOrGroupNotFound(pkg.to_string()))?;

                let ManifestPackageDescriptor::Catalog(Self::CatalogDescriptor {
                    pkg_group, ..
                }) = descriptor
                else {
                    return Ok(None);
                };

                let Some(group) = pkg_group else {
                    return Ok(None);
                };
                let pkgs = self.pkg_descriptors_in_named_group(group);
                let other_pkgs_in_group = pkgs.iter().any(|(id, _)| id != pkg);
                if other_pkgs_in_group {
                    Ok(Some(group.clone()))
                } else {
                    Ok(None)
                }
            }

            /// Check whether the specified package belongs to the "toplevel" group
            /// with additional packages.
            fn pkg_belongs_to_non_empty_toplevel_group(
                &self,
                pkg: impl AsRef<str>,
            ) -> Result<bool, ManifestError> {
                let descriptors = self.install.inner();
                let pkg = pkg.as_ref();
                descriptors
                    .get(pkg)
                    .ok_or(ManifestError::PkgOrGroupNotFound(pkg.to_string()))?;
                let pkgs = self.pkg_descriptors_in_toplevel_group();
                let self_in_toplevel_group = pkgs.iter().any(|(id, _)| id == pkg);
                let other_toplevel_packages_exist = pkgs.iter().any(|(id, _)| id != pkg);
                Ok(self_in_toplevel_group && other_toplevel_packages_exist)
            }

            /// Resolve "loose" package references (e.g. pkg-paths),
            /// to `install_ids` if unambiguous
            /// so that installation references remain valid for other package operations.
            fn get_install_ids(&self, packages: Vec<String>) -> Result<Vec<String>, ManifestError> {
                let mut install_ids = Vec::new();
                for pkg in packages {
                    // User passed an install id directly
                    if self.install.inner().contains_key(&pkg) {
                        install_ids.push(pkg);
                        continue;
                    }

                    // User passed a package path to uninstall
                    // To support version constraints, we match the provided value against
                    // `<pkg-path>` and `<pkg-path>@<version>`.
                    let matching_iids_by_pkg_path = self
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
                        0 => return Err(ManifestError::PackageNotFound(pkg)),
                        // if there is only one package with the given pkg-path, uninstall it
                        1 => install_ids.extend(matching_iids_by_pkg_path),
                        // if there are multiple packages with the given pkg-path, ask for a specific install id
                        _ => {
                            return Err(ManifestError::MultiplePackagesMatch(
                                pkg,
                                matching_iids_by_pkg_path,
                            ));
                        },
                    }
                }
                Ok(install_ids)
            }
        }
    };
}
pub(crate) use impl_pkg_lookup;
