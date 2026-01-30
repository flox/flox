use crate::parsed::v1_9_0::{self as man, package_descriptor as pkgs};

pub type ManifestLatest = man::ManifestV1_9_0;
pub type Install = man::Install;
pub type ManifestPackageDescriptor = pkgs::ManifestPackageDescriptor;
pub type PackageDescriptorCatalog = pkgs::PackageDescriptorCatalog;
pub type PackageDescriptorFlake = pkgs::PackageDescriptorFlake;
pub type SelectedOutputs = pkgs::SelectedOutputs;
pub type AllSentinel = pkgs::AllSentinel;
