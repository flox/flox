pub mod common;
pub mod latest;
pub mod v1;
pub mod v1_9_0;

/// An interface codifying how to access types that are just semantic wrappers
/// around inner types. This impl may be generated with a macro.
pub trait Inner {
    type Inner;

    fn inner(&self) -> &Self::Inner;
    fn inner_mut(&mut self) -> &mut Self::Inner;
    fn into_inner(self) -> Self::Inner;
}

/// A macro that generates a `Inner` impl.
macro_rules! impl_into_inner {
    ($wrapper:ty, $inner_type:ty) => {
        impl crate::parsed::Inner for $wrapper {
            type Inner = $inner_type;

            fn inner(&self) -> &Self::Inner {
                &self.0
            }

            fn inner_mut(&mut self) -> &mut Self::Inner {
                &mut self.0
            }

            fn into_inner(self) -> Self::Inner {
                self.0
            }
        }
    };
}

pub(crate) use impl_into_inner;

/// An interface for the type of function that serde's skip_serializing_if
/// method takes.
pub(crate) trait SkipSerializing {
    fn skip_serializing(&self) -> bool;
}

// pub trait PackageLookup {
//     type PkgDescriptor;
//     type CatalogDescriptor;
//     type FlakeDescriptor;

//     fn pkg_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<Self::PkgDescriptor>;
//     fn catalog_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<Self::CatalogDescriptor>;
//     fn flake_pkg_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<Self::PkgDescriptor>;
//     fn pkg_descriptors_in_toplevel_group(&self) -> Vec<(String, Self::PkgDescriptor)>;
//     fn pkg_descriptors_in_named_group(
//         &self,
//         name: impl AsRef<str>,
//     ) -> Vec<(String, Self::PkgDescriptor)>;
//     fn pkg_or_group_found_in_manifest(&self, name: impl AsRef<str>) -> bool;
//     fn pkg_belongs_to_non_empty_named_group(
//         &self,
//         pkg: impl AsRef<str>,
//     ) -> Result<Option<String>, ManifestError>;
//     fn pkg_belongs_to_non_empty_toplevel_group(
//         &self,
//         pkg: impl AsRef<str>,
//     ) -> Result<bool, ManifestError>;
//     fn get_install_ids(&self, packages: Vec<String>) -> Result<Vec<String>, ManifestError>;
// }

// macro_rules! impl_pkg_lookup {
//     ($manifest_type:ty, $pkg_descriptor_type:ty, $catalog_descriptor_type:ty, $flake_descriptor_type:ty) => {
//         impl crate::parsed::PackageLookup for $manifest_type {
//             type CatalogDescriptor = $catalog_descriptor_type;
//             type FlakeDescriptor = $flake_descriptor_type;
//             type PkgDescriptor = $pkg_descriptor_type;

//             /// Get the package descriptor with the specified install_id.
//             fn pkg_descriptor_with_id(
//                 &self,
//                 id: impl AsRef<str>,
//             ) -> Option<Self::PkgDescriptor> {
//                 self.install.0.get(id.as_ref()).cloned()
//             }

//             /// Get the package descriptor with the specified install_id.
//             fn catalog_pkg_descriptor_with_id(
//                 &self,
//                 id: impl AsRef<str>,
//             ) -> Option<Self::CatalogDescriptor> {
//                 self.install
//                     .0
//                     .get(id.as_ref())
//                     .and_then(ManifestPackageDescriptor::as_catalog_descriptor_ref)
//                     .cloned()
//             }

//             /// Get the package descriptor with the specified install_id.
//             fn flake_pkg_descriptor_with_id(
//                 &self,
//                 id: impl AsRef<str>,
//             ) -> Option<ManifestPackageDescriptor> {
//                 self.install.0.get(id.as_ref()).cloned()
//             }

//             /// Get the package descriptors in the "toplevel" group.
//             fn pkg_descriptors_in_toplevel_group(
//                 &self,
//             ) -> Vec<(String, ManifestPackageDescriptor)> {
//                 pkg_descriptors_in_toplevel_group(&self.install.0)
//             }

//             /// Get the package descriptors in a named group.
//             fn pkg_descriptors_in_named_group(
//                 &self,
//                 name: impl AsRef<str>,
//             ) -> Vec<(String, ManifestPackageDescriptor)> {
//                 pkg_descriptors_in_named_group(name, &self.install.0)
//             }

//             /// Check whether the specified name is either an install_id or group name.
//             fn pkg_or_group_found_in_manifest(&self, name: impl AsRef<str>) -> bool {
//                 pkg_or_group_found_in_manifest(name.as_ref(), &self.install.0)
//             }

//             /// Check whether the specified package belongs to a named group
//             /// with additional packages.
//             fn pkg_belongs_to_non_empty_named_group(
//                 &self,
//                 pkg: impl AsRef<str>,
//             ) -> Result<Option<String>, ManifestError> {
//                 pkg_belongs_to_non_empty_named_group(pkg.as_ref(), &self.install.0)
//             }

//             /// Check whether the specified package belongs to the "toplevel" group
//             /// with additional packages.
//             fn pkg_belongs_to_non_empty_toplevel_group(
//                 &self,
//                 pkg: impl AsRef<str>,
//             ) -> Result<bool, ManifestError> {
//                 pkg_belongs_to_non_empty_toplevel_group(pkg.as_ref(), &self.install.0)
//             }

//             /// Resolve "loose" package references (e.g. pkg-paths),
//             /// to `install_ids` if unambiguous
//             /// so that installation references remain valid for other package operations.
//             fn get_install_ids(&self, packages: Vec<String>) -> Result<Vec<String>, ManifestError> {
//                 let mut install_ids = Vec::new();
//                 for pkg in packages {
//                     // User passed an install id directly
//                     if self.install.inner().contains_key(&pkg) {
//                         install_ids.push(pkg);
//                         continue;
//                     }

//                     // User passed a package path to uninstall
//                     // To support version constraints, we match the provided value against
//                     // `<pkg-path>` and `<pkg-path>@<version>`.
//                     let matching_iids_by_pkg_path = self
//                         .install
//                         .inner()
//                         .iter()
//                         .filter(|(_iid, descriptor)| {
//                             // Find matching pkg-paths and select for uninstall

//                             // If the descriptor is not a catalog descriptor, skip.
//                             // flakes descriptors are only matched by install_id.
//                             let ManifestPackageDescriptor::Catalog(des) = descriptor else {
//                                 return false;
//                             };

//                             // Select if the descriptor's pkg_path matches the user's input
//                             if des.pkg_path == pkg {
//                                 return true;
//                             }

//                             // Select if the descriptor matches the user's input when the version is included
//                             // Future: if we want to allow uninstalling a specific outputs as well,
//                             //         parsing of uninstall specs will need to be more sophisticated.
//                             //         For now going with a simple check for pkg-path@version.
//                             if let Some(version) = &des.version {
//                                 format!("{}@{}", des.pkg_path, version) == pkg
//                             } else {
//                                 false
//                             }
//                         })
//                         .map(|(iid, _)| iid.to_owned())
//                         .collect::<Vec<String>>();

//                     // Extend the install_ids with the matching install id from pkg-path
//                     match matching_iids_by_pkg_path.len() {
//                         0 => return Err(ManifestError::PackageNotFound(pkg)),
//                         // if there is only one package with the given pkg-path, uninstall it
//                         1 => install_ids.extend(matching_iids_by_pkg_path),
//                         // if there are multiple packages with the given pkg-path, ask for a specific install id
//                         _ => {
//                             return Err(ManifestError::MultiplePackagesMatch(
//                                 pkg,
//                                 matching_iids_by_pkg_path,
//                             ));
//                         },
//                     }
//                 }
//                 Ok(install_ids)
//             }
//         }
//     };
// }
// pub(crate) use impl_pkg_lookup;

// pub(crate) fn pkg_descriptors_in_toplevel_group(
//     descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
// ) -> Vec<(String, ManifestPackageDescriptor)> {
//     descriptors
//         .iter()
//         .filter(|(_, desc)| {
//             let ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog { pkg_group, .. }) =
//                 desc
//             else {
//                 return false;
//             };

//             pkg_group.is_none()
//         })
//         .map(|(id, desc)| (id.clone(), desc.clone()))
//         .collect::<Vec<_>>()
// }

// pub(crate) fn pkg_descriptors_in_named_group(
//     name: impl AsRef<str>,
//     descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
// ) -> Vec<(String, ManifestPackageDescriptor)> {
//     descriptors
//         .iter()
//         .filter(|(_, desc)| {
//             let ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog { pkg_group, .. }) =
//                 desc
//             else {
//                 return false;
//             };

//             pkg_group
//                 .as_ref()
//                 .is_some_and(|n| n.as_str() == name.as_ref())
//         })
//         .map(|(id, desc)| (id.clone(), desc.clone()))
//         .collect::<Vec<_>>()
// }

// /// Scans the provided package descriptors to determine if the search term is a package or
// /// group in the manifest.
// fn pkg_or_group_found_in_manifest(
//     search_term: impl AsRef<str>,
//     descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
// ) -> bool {
//     descriptors.iter().any(|(id, desc)| {
//         let group = if let ManifestPackageDescriptor::Catalog(catalog) = desc {
//             catalog.pkg_group.as_deref()
//         } else {
//             None
//         };

//         let search_term = search_term.as_ref();

//         (search_term == id.as_str()) || (Some(search_term) == group)
//     })
// }

// /// named group in the manifest with other packages.
// fn pkg_belongs_to_non_empty_named_group(
//     pkg: &str,
//     descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
// ) -> Result<Option<String>, ManifestError> {
//     let descriptor = descriptors
//         .get(pkg)
//         .ok_or(ManifestError::PkgOrGroupNotFound(pkg.to_string()))?;

//     let ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog { pkg_group, .. }) = descriptor
//     else {
//         return Ok(None);
//     };

//     let Some(group) = pkg_group else {
//         return Ok(None);
//     };
//     let pkgs = pkg_descriptors_in_named_group(group, descriptors);
//     let other_pkgs_in_group = pkgs.iter().any(|(id, _)| id != pkg);
//     if other_pkgs_in_group {
//         Ok(Some(group.clone()))
//     } else {
//         Ok(None)
//     }
// }

// /// Scans the provided package descriptors to determine if the specified package belongs to
// /// the "toplevel" group with other packages.
// fn pkg_belongs_to_non_empty_toplevel_group(
//     pkg: &str,
//     descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
// ) -> Result<bool, ManifestError> {
//     descriptors
//         .get(pkg)
//         .ok_or(ManifestError::PkgOrGroupNotFound(pkg.to_string()))?;
//     let pkgs = pkg_descriptors_in_toplevel_group(descriptors);
//     let self_in_toplevel_group = pkgs.iter().any(|(id, _)| id == pkg);
//     let other_toplevel_packages_exist = pkgs.iter().any(|(id, _)| id != pkg);
//     Ok(self_in_toplevel_group && other_toplevel_packages_exist)
// }
