use flox_manifest::interfaces::{AsLatestSchema, PackageLookup};
use flox_manifest::lockfile::Lockfile;
use flox_manifest::parsed::common::DEFAULT_GROUP_NAME;
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

/// Check that each string in `requested` appears in `all_outputs`.
///
/// Returns `Err(InvalidOutputForPackage(output, install_id))` for the first
/// output that is not present.  Used by both the merge-path check inside
/// `compute_install_modification` and the add-path check in
/// `validate_outputs_against_lockfile` so the error message is identical in
/// both cases.
fn check_outputs_are_valid(
    requested: &[String],
    all_outputs: &[String],
    install_id: &str,
) -> Result<(), InstallOrUninstallError> {
    for output in requested {
        if !all_outputs.contains(output) {
            return Err(InstallOrUninstallError::InvalidOutputForPackage(
                output.clone(),
                install_id.to_string(),
            ));
        }
    }
    Ok(())
}

/// Validate that every `RawSelectedOutputs::Specific` output requested by
/// each package actually exists in the package's resolved `all_outputs`.
///
/// Called on the **add path** after the manifest has been modified and
/// re-locked, using the fresh lockfile that contains `all_outputs` for
/// newly-added packages.  Packages with `outputs = None` or
/// `RawSelectedOutputs::All` are skipped — they cannot be invalid.
///
/// Returns `Err(InvalidOutputForPackage(output, install_id))` for the first
/// invalid output found, matching the error produced by the merge-path check
/// inside `compute_install_modification`.
pub(super) fn validate_outputs_against_lockfile(
    packages: &[PackageToInstall],
    lockfile: &Lockfile,
) -> Result<(), InstallOrUninstallError> {
    for pkg in packages {
        // Skip packages with no output selection or All-outputs — only
        // Specific outputs can be invalid.
        let Some(RawSelectedOutputs::Specific(requested)) = pkg.outputs() else {
            continue;
        };
        let install_id = pkg.id();
        let Some(locked_pkg) = lockfile.locked_package_with_id(install_id) else {
            // Package not resolved (e.g. store-path or unsupported type) — skip.
            continue;
        };
        check_outputs_are_valid(requested, &locked_pkg.all_outputs(), install_id)?;
    }
    Ok(())
}

/// Compute all modifications needed to install the given packages.
///
/// Errors for invalid requests and filters out no-ops,
/// so the returned Vec<PackageToModify> is a validated list of changes to make
pub(super) fn compute_install_modifications(
    packages: &[PackageToInstall],
    manifest: &Manifest<Migrated>,
    lockfile: &Lockfile,
) -> Result<Vec<PackageToModify>, InstallOrUninstallError> {
    for pkg in packages {
        check_stability_conflict(pkg, manifest)?;
    }

    let modifications = packages
        .iter()
        .filter_map(|pkg| compute_install_modification(pkg, manifest, lockfile).transpose())
        .collect::<Result<Vec<_>, _>>()?;

    debug!(?modifications, "computed install modifications");
    Ok(modifications)
}

/// Refuse to change the stability of a group that already has packages.
///
/// A group resolves against a single catalog page, so its stability applies
/// to every package in it. Changing it as a side effect of installing one
/// package would silently re-resolve the others.
fn check_stability_conflict(
    pkg: &PackageToInstall,
    manifest: &Manifest<Migrated>,
) -> Result<(), InstallOrUninstallError> {
    let PackageToInstall::Catalog(catalog_pkg) = pkg else {
        return Ok(());
    };
    let Some(requested) = &catalog_pkg.stability else {
        return Ok(());
    };
    let group = catalog_pkg
        .target_group()
        .unwrap_or_else(|| DEFAULT_GROUP_NAME.to_string());
    let manifest = manifest.as_latest_schema();
    let current = manifest.group_stability(&group);
    if current == Some(requested.as_str()) || !manifest.group_has_packages(&group) {
        return Ok(());
    }
    Err(InstallOrUninstallError::StabilityConflict {
        current: current.map(str::to_string),
        requested: requested.clone(),
        group,
    })
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
            // Validate all requested outputs before modifying `merged` so the
            // error message is consistent with the add-path check.
            check_outputs_are_valid(requested, &all_outputs, install_id)?;
            for output in requested {
                if !merged.contains(output) {
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
    use flox_manifest::raw::test_helpers::{
        empty_test_migrated_manifest,
        mk_test_manifest_from_contents,
    };
    use flox_manifest::test_helpers::with_latest_schema;
    use flox_test_utils::GENERATED_DATA;
    use indoc::indoc;
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
            pkg_group: None,
            stability: None,
        })
    }

    fn package_to_install_with_stability(
        pkg_path: &str,
        pkg_group: Option<&str>,
        stability: &str,
    ) -> PackageToInstall {
        let PackageToInstall::Catalog(mut pkg) = package_to_install(pkg_path, pkg_path, None)
        else {
            unreachable!()
        };
        pkg.pkg_group = pkg_group.map(str::to_string);
        pkg.stability = Some(stability.to_string());
        PackageToInstall::Catalog(pkg)
    }

    /// Install `pkg` into a manifest that has `hello` in the default group
    /// and `curl` in the `tools` group, which uses the `stable` stability.
    fn stability_check(pkg: PackageToInstall) -> Result<(), InstallOrUninstallError> {
        let manifest = mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
            [install]
            hello.pkg-path = "hello"
            curl.pkg-path = "curl"
            curl.pkg-group = "tools"

            [pkg-groups.tools]
            stability = "stable"
        "#}));
        check_stability_conflict(&pkg, &manifest)
    }

    #[test]
    fn stability_for_new_group_is_accepted() {
        let pkg = package_to_install_with_stability("jq", Some("new"), "lts");
        assert!(stability_check(pkg).is_ok());
    }

    #[test]
    fn stability_matching_group_is_accepted() {
        let pkg = package_to_install_with_stability("jq", Some("tools"), "stable");
        assert!(stability_check(pkg).is_ok());
    }

    #[test]
    fn stability_differing_from_group_is_rejected() {
        let pkg = package_to_install_with_stability("jq", Some("tools"), "lts");
        let Err(InstallOrUninstallError::StabilityConflict {
            group,
            current,
            requested,
        }) = stability_check(pkg)
        else {
            panic!("expected a stability conflict");
        };
        assert_eq!(
            (group, current, requested),
            (
                "tools".to_string(),
                Some("stable".to_string()),
                "lts".to_string()
            )
        );
    }

    /// Packages without `--pkg-group` target the default group, which here
    /// has packages but no explicit stability.
    #[test]
    fn stability_for_default_group_with_packages_is_rejected() {
        let pkg = package_to_install_with_stability("jq", None, "lts");
        let Err(InstallOrUninstallError::StabilityConflict {
            group,
            current,
            requested,
        }) = stability_check(pkg)
        else {
            panic!("expected a stability conflict");
        };
        assert_eq!(
            (group, current, requested),
            ("toplevel".to_string(), None, "lts".to_string())
        );
    }

    /// Custom catalog packages get a group of their own, so a stability never
    /// conflicts with the default group.
    #[test]
    fn stability_for_custom_catalog_package_is_accepted() {
        let pkg = package_to_install_with_stability("myorg/mypkg", None, "lts");
        assert!(stability_check(pkg).is_ok());
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

    // For a package on the add path (not yet in the manifest), requesting an
    // output that doesn't exist in the resolved package should return
    // `InvalidOutputForPackage`.
    //
    // This mirrors the merge-path test for invalid outputs, but uses an empty
    // manifest so the package goes through the Add branch rather than the
    // merge (UpdateOutputs) branch.
    #[test]
    fn invalid_output_on_add_path_returns_error() {
        // Use a lockfile that already has bash resolved (with known outputs) so
        // we can validate against `all_outputs` without a real catalog round-trip.
        let (_, lockfile) = load_manifest_and_lockfile("bash_v1_10_0_out");

        // Empty manifest — bash is NOT installed yet, so this is an add-path install.
        let manifest = flox_manifest::raw::test_helpers::empty_test_migrated_manifest();

        let pkg = package_to_install(
            "bash",
            "bashNonInteractive",
            Some(RawSelectedOutputs::Specific(vec!["bad".to_string()])),
        );

        let err =
            validate_outputs_against_lockfile(std::slice::from_ref(&pkg), &lockfile).unwrap_err();
        assert!(
            matches!(
                err,
                InstallOrUninstallError::InvalidOutputForPackage(ref output, ref id)
                    if output == "bad" && id == "bash"
            ),
            "expected InvalidOutputForPackage(\"bad\", \"bash\"), got: {err:?}"
        );

        // The compute_install_modifications call itself does NOT yet catch this
        // (it just produces an Add modification); the error surfaces via the
        // post-lock validate_outputs_against_lockfile call in core_environment.
        let result =
            compute_install_modifications(std::slice::from_ref(&pkg), &manifest, &lockfile)
                .unwrap();
        assert_eq!(result, vec![flox_manifest::raw::PackageToModify {
            install_id: "bash".to_string(),
            modification: flox_manifest::raw::PackageModification::Add(pkg),
        }]);
    }
}
