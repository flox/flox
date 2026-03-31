use flox_manifest::interfaces::PackageLookup;
use flox_manifest::lockfile::Lockfile;
use flox_manifest::parsed::latest::{AllSentinel, SelectedOutputs};
use flox_manifest::raw::{
    PackageModification,
    PackageToInstall,
    PackageToModify,
    RawSelectedOutputs,
};
use flox_manifest::{Manifest, Migrated};
use tracing::debug;

use crate::models::environment::InstallOrUninstallError;

/// Compute all modifications needed to install the given packages.
///
/// Errors for invalid requests and filters out no-ops,
/// so the returned Vec<PackageToModify> is a validated list of changes to make
pub(super) fn compute_install_modifications(
    packages: &[PackageToInstall],
    manifest: &Manifest<Migrated>,
    lockfile: &Lockfile,
) -> Result<Vec<PackageToModify>, InstallOrUninstallError> {
    let modifications = packages
        .iter()
        .filter_map(|pkg| compute_install_modification(pkg, manifest, lockfile).transpose())
        .collect::<Result<Vec<_>, _>>()?;

    debug!(?modifications, "computed install modifications");
    Ok(modifications)
}

/// Compute the modification (if any) needed to install a single package.
///
/// Returns `Ok(None)` when the package is already installed
pub(super) fn compute_install_modification(
    pkg: &PackageToInstall,
    manifest: &Manifest<Migrated>,
    lockfile: &Lockfile,
) -> Result<Option<PackageToModify>, InstallOrUninstallError> {
    let install_id = pkg.id();

    // We don't check whether the package is already installed via an include.
    // We just install the package as an override and later warn in the CLI

    let Some(manifest_descriptor) = manifest.pkg_descriptor_with_id(install_id) else {
        // Package is not yet in the manifest — add it.
        return Ok(Some(PackageToModify {
            install_id: install_id.to_string(),
            modification: PackageModification::Add(pkg.clone()),
        }));
    };

    // Package is already installed. Check whether outputs need merging.

    // TODO: outputs of a package could change if a package gets re-resolved to a different version,
    // but we'll ignore that as an edge case for now
    let requested_outputs = pkg.outputs();
    let current_outputs = manifest_descriptor.get_outputs();

    match (current_outputs, requested_outputs) {
        // When no outputs are requested for an already installed package, do nothing
        (_, None) => Ok(None),
        // If all outputs are already installed, do nothing
        (Some(SelectedOutputs::All(_)), _) => Ok(None),
        // If all outputs are requested, set outputs to all
        (_, Some(RawSelectedOutputs::All)) => Ok(Some(PackageToModify {
            install_id: install_id.to_string(),
            modification: PackageModification::UpdateOutputs(SelectedOutputs::All(
                AllSentinel::All,
            )),
        })),
        // In all other cases, merge current and requested outputs
        (current_outputs, Some(RawSelectedOutputs::Specific(requested))) => {
            // Determine effective current outputs from manifest or
            // lockfile defaults (what the resolver originally chose).
            let locked_pkg = lockfile.locked_package_with_id(install_id).ok_or_else(|| {
                InstallOrUninstallError::PackageInManifestNotInLockfile(install_id.to_string())
            })?;

            let effective_current: Vec<String> = match current_outputs {
                // This will lead to weird behavior if e.g:
                // outputs_to_install = None
                // requested = ["lib"]
                // and buildenv is currently defaulting to ["out"]
                // We'll go from having ["out"] installed to ["lib"]
                // That's pretty unlikely because nixpkgs `stdenv`
                // auto-populates `meta.outputsToInstall` for any package built
                // via `stdenv.mkDerivation`.
                // From `pkgs/stdenv/generic/check-meta.nix`:
                //
                // ```nix
                // outputsToInstall = [
                //   (if hasOutput "bin" then "bin"
                //    else if hasOutput "out" then "out"
                //    else findFirst hasOutput null outputs)
                // ] ++ optional (hasOutput "man") "man";
                // ```
                //
                // So every stdenv-built package gets at minimum `["out"]` (or
                // `["bin"]`, plus `"man"` when present). To produce `null` you
                // need one of:
                //
                // - A non-stdenv derivation that bypasses `commonMeta`
                // - A package that explicitly sets `meta.outputsToInstall = null;`
                // - A catalog ingestion bug
                None => locked_pkg.outputs_to_install().unwrap_or_default(),
                Some(SelectedOutputs::Specific(list)) => list.clone(),
                Some(SelectedOutputs::All(_)) => unreachable!(),
            };

            // Union: current + requested, preserving order and
            // uniqueness.
            let mut merged = effective_current.clone();
            let mut added_output = false;
            let all_outputs = locked_pkg.all_outputs();
            for output in requested {
                if !merged.contains(output) {
                    if !all_outputs.contains(output) {
                        return Err(InstallOrUninstallError::InvalidOutputForPackage(
                            output.to_string(),
                            install_id.to_string(),
                        ));
                    }
                    merged.push(output.clone());
                    added_output = true;
                }
            }

            if added_output {
                Ok(Some(PackageToModify {
                    install_id: install_id.to_string(),
                    modification: PackageModification::UpdateOutputs(SelectedOutputs::Specific(
                        merged,
                    )),
                }))
            } else {
                Ok(None)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use flox_core::canonical_path::CanonicalPath;
    use flox_manifest::raw::CatalogPackage;
    use flox_manifest::raw::test_helpers::empty_test_migrated_manifest;
    use flox_test_utils::GENERATED_DATA;
    use pretty_assertions::assert_eq;

    use super::*;

    /// Load a `Manifest<Migrated>` and `Lockfile` from a generated env directory.
    fn load_manifest_and_lockfile(env_subdir: &str) -> (Manifest<Migrated>, Lockfile) {
        let env_dir = GENERATED_DATA.join(Path::new("envs").join(env_subdir));
        let manifest_path = env_dir.join("manifest.toml");
        let lockfile_path = env_dir.join("manifest.lock");
        let manifest = Manifest::read_and_migrate(&manifest_path, &lockfile_path).unwrap();
        let lockfile =
            Lockfile::read_from_file(&CanonicalPath::new(&lockfile_path).unwrap()).unwrap();
        (manifest, lockfile)
    }

    fn package_to_install(
        id: &str,
        pkg_path: &str,
        outputs: Option<RawSelectedOutputs>,
    ) -> PackageToInstall {
        PackageToInstall::Catalog(CatalogPackage {
            id: id.to_string(),
            pkg_path: pkg_path.to_string(),
            version: None,
            systems: None,
            outputs,
        })
    }

    // For an empty manifest
    // `install bashNonInteractive -i bash`
    // installs bashNonInteractive
    #[test]
    fn add_new_packages() {
        let manifest = empty_test_migrated_manifest();
        let lockfile = Lockfile::default();
        let pkg = package_to_install("bash", "bashNonInteractive", None);

        let result =
            compute_install_modifications(std::slice::from_ref(&pkg), &manifest, &lockfile)
                .unwrap();

        assert_eq!(result, vec![PackageToModify {
            install_id: "bash".to_string(),
            modification: PackageModification::Add(pkg),
        }]);
    }

    // If manifest has `bash.outputs = ["out"]`
    // `install bashNonInteractive -i bash`
    // is a no-op
    #[test]
    fn pkg_already_installed_no_outputs_requested_is_noop() {
        let (manifest, lockfile) = load_manifest_and_lockfile("bash_v1_10_0_out");
        let pkg = package_to_install("bash", "bashNonInteractive", None);

        let result = compute_install_modifications(&[pkg], &manifest, &lockfile).unwrap();

        assert_eq!(result, Vec::new());
    }

    // If manifest has `bash.outputs = ["out"]`
    // `install bashNonInteractive^.. -i bash`
    // updates outputs to "all"
    #[test]
    fn install_all_outputs_updates_manifest() {
        let (manifest, lockfile) = load_manifest_and_lockfile("bash_v1_10_0_out");
        let pkg = package_to_install("bash", "bashNonInteractive", Some(RawSelectedOutputs::All));

        let result = compute_install_modifications(&[pkg], &manifest, &lockfile).unwrap();

        assert_eq!(result, vec![PackageToModify {
            install_id: "bash".to_string(),
            modification: PackageModification::UpdateOutputs(SelectedOutputs::All(
                AllSentinel::All
            )),
        }]);
    }

    // If manifest has `bash.outputs = ["out"]`
    // `install bashNonInteractive^out -i bash`
    // is a no-op
    #[test]
    fn request_outputs_already_installed_is_noop() {
        let (manifest, lockfile) = load_manifest_and_lockfile("bash_v1_10_0_out");
        let pkg = package_to_install(
            "bash",
            "bashNonInteractive",
            Some(RawSelectedOutputs::Specific(vec!["out".to_string()])),
        );

        let result = compute_install_modifications(&[pkg], &manifest, &lockfile).unwrap();

        assert_eq!(result, Vec::new());
    }
}
