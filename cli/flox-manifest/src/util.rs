/// Custom packages are of the form "<prefix>/<suffix>" where the prefix is not
/// allowed to contain a '.' character. This is a quick and dirty way of
/// identifying custom packages using that logic.
///
/// Favour using CatalogPackage::is_custom_catalog if you already have a CatalogPackage
pub fn is_custom_package(pkg_path: &str) -> bool {
    let parts: Vec<&str> = pkg_path.split('/').collect();
    let is_base_catalog_pkg = parts.len() == 1 || parts.first().is_some_and(|p| p.contains('.'));
    !is_base_catalog_pkg
}
