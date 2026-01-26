use crate::interfaces::pkg_lookup::PackageLookup;
use crate::parsed::latest;
use crate::{Manifest, Migrated};

impl PackageLookup for Manifest<Migrated> {
    type CatalogDescriptor = latest::PackageDescriptorCatalog;
    type FlakeDescriptor = latest::PackageDescriptorFlake;
    type PkgDescriptor = latest::ManifestPackageDescriptor;

    fn catalog_pkgs_by_group(
        &self,
    ) -> std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, Self::CatalogDescriptor>,
    > {
        self.inner.migrated_parsed.catalog_pkgs_by_group()
    }

    fn pkg_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<Self::PkgDescriptor> {
        self.inner.migrated_parsed.pkg_descriptor_with_id(id)
    }

    fn catalog_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<Self::CatalogDescriptor> {
        self.inner.migrated_parsed.catalog_descriptor_with_id(id)
    }

    fn flake_pkg_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<Self::PkgDescriptor> {
        self.inner.migrated_parsed.flake_pkg_descriptor_with_id(id)
    }

    fn pkg_descriptors_in_toplevel_group(&self) -> Vec<(String, Self::PkgDescriptor)> {
        self.inner
            .migrated_parsed
            .pkg_descriptors_in_toplevel_group()
    }

    fn pkg_descriptors_in_named_group(
        &self,
        name: impl AsRef<str>,
    ) -> Vec<(String, Self::PkgDescriptor)> {
        self.inner
            .migrated_parsed
            .pkg_descriptors_in_named_group(name)
    }

    fn pkg_or_group_found_in_manifest(&self, name: impl AsRef<str>) -> bool {
        self.inner
            .migrated_parsed
            .pkg_or_group_found_in_manifest(name)
    }

    fn pkg_belongs_to_non_empty_named_group(
        &self,
        pkg: impl AsRef<str>,
    ) -> Result<Option<String>, crate::ManifestError> {
        self.inner
            .migrated_parsed
            .pkg_belongs_to_non_empty_named_group(pkg)
    }

    fn pkg_belongs_to_non_empty_toplevel_group(
        &self,
        pkg: impl AsRef<str>,
    ) -> Result<bool, crate::ManifestError> {
        self.inner
            .migrated_parsed
            .pkg_belongs_to_non_empty_toplevel_group(pkg)
    }

    fn get_install_ids(&self, packages: Vec<String>) -> Result<Vec<String>, crate::ManifestError> {
        self.inner.migrated_parsed.get_install_ids(packages)
    }
}
