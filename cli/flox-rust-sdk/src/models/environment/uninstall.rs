use std::collections::HashMap;
use std::collections::hash_map::Entry;

use flox_manifest::interfaces::PackageLookup;
use flox_manifest::lockfile::Lockfile;
use flox_manifest::parsed::v1_10_0::SelectedOutputs;
use flox_manifest::raw::{
    CatalogPackage,
    PackageModification,
    PackageToModify,
    RawManifestError,
    RawSelectedOutputs,
};
use flox_manifest::{Manifest, ManifestError, Migrated};
use reqwest::Url;
use tracing::debug;

use super::UninstallError;

/// A specification for what to uninstall.
///
/// Can represent a full package removal or selective output removal.
#[derive(Debug, Clone, PartialEq)]
pub struct UninstallSpec {
    /// The package reference (install_id or pkg_path).
    pub package_ref: String,
    /// If Some, only remove these specific outputs rather than the entire package.
    pub outputs: Option<RawSelectedOutputs>,
    /// Optional version constraint for disambiguation.
    pub version: Option<String>,
}

impl UninstallSpec {
    /// Parse an uninstall spec from a CLI argument string.
    ///
    /// Reuses the same grammar as `CatalogPackage::from_str`:
    /// - `hello` — remove the package
    /// - `hello^man,doc` — remove specific outputs
    /// - `hello^..` — remove all outputs (equivalent to full removal)
    /// - `hello@1.2.3` — remove a specific version
    ///
    /// For URL inputs (flake refs), the entire string is kept as `package_ref`
    /// with no outputs/version.
    pub fn parse(s: &str) -> Result<Self, RawManifestError> {
        match Url::parse(s) {
            Ok(url) => Ok(UninstallSpec {
                package_ref: url.to_string(),
                outputs: None,
                version: None,
            }),
            _ => {
                let CatalogPackage {
                    id: _id,
                    pkg_path,
                    version,
                    systems: _systems,
                    outputs,
                } = s.parse()?;

                Ok(UninstallSpec {
                    package_ref: pkg_path,
                    outputs,
                    version,
                })
            },
        }
    }
}

/// Resolve uninstall specifications to PackagesToModify.
///
/// This function processes a list of uninstall specs and:
/// 1. Resolves each package reference (pkg-path or install_id) to a concrete install_id
/// 2. Aggregates outputs to remove when multiple specs target the same package
/// 3. Returns detailed errors if packages are only available in includes
/// 4. Validates the specified outputs exist for the package and computes the
///    unnecessary modifications
pub fn resolve_specs_to_modifications(
    specs: &[UninstallSpec],
    manifest: &Manifest<Migrated>,
    lockfile: &Lockfile,
) -> Result<Vec<PackageToModify>, UninstallError> {
    let mut removals = HashMap::new();

    for spec in specs {
        // Resolve the package reference to an install_id
        let install_id = match manifest.resolve_install_id(&spec.package_ref, &spec.version) {
            Ok(id) => id,
            Err(ManifestError::PackageNotFound(ref pkg)) => {
                // If package wasn't found in manifest, check if it exists only in an include
                if let Some(include) = lockfile
                    .compose
                    .as_ref()
                    .map(|c| c.get_include_for_package(pkg, &spec.version))
                    .transpose()?
                    .flatten()
                {
                    return Err(UninstallError::PackageOnlyIncluded(
                        pkg.clone(),
                        include.name,
                    ));
                }
                return Err(ManifestError::PackageNotFound(pkg.clone()).into());
            },
            Err(e) => return Err(e.into()),
        };

        let outputs_to_uninstall = spec
            .outputs
            .as_ref()
            .unwrap_or(&RawSelectedOutputs::All)
            .clone();

        // Aggregate outputs to remove if multiple specs target the same package
        let outputs_to_remove_accumulated = removals.entry(install_id);
        match outputs_to_remove_accumulated {
            Entry::Occupied(mut occupied_entry) => {
                let accumulated = occupied_entry.get_mut();
                match (accumulated, outputs_to_uninstall) {
                    // If either the accumulated or new spec removes all outputs, remove all
                    (accumulated @ RawSelectedOutputs::All, _)
                    | (accumulated, RawSelectedOutputs::All) => {
                        *accumulated = RawSelectedOutputs::All;
                    },
                    // Otherwise, merge the specific output lists
                    (
                        RawSelectedOutputs::Specific(items_acc),
                        RawSelectedOutputs::Specific(items),
                    ) => items_acc.extend(items),
                };
            },
            Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(outputs_to_uninstall);
            },
        };
    }
    compute_uninstall_modifications(removals, manifest, lockfile)
}

/// Compute manifest modifications to uninstall the specified outputs from packages.
///
/// For each package in `removals`, this function:
/// 1. Extracts the current outputs configuration from the manifest
/// 2. Retrieves available outputs and currently installed outputs from the lockfile
/// 3. Validates that requested outputs exist for the package
/// 4. Computes the appropriate modification (remove package entirely or update outputs)
fn compute_uninstall_modifications(
    removals: impl IntoIterator<Item = (String, RawSelectedOutputs)>,
    manifest: &Manifest<Migrated>,
    lockfile: &Lockfile,
) -> Result<Vec<PackageToModify>, UninstallError> {
    let mut modifications = Vec::new();

    for (install_id, outputs_to_uninstall) in removals {
        // Get current manifest outputs
        let manifest_descriptor = manifest.pkg_descriptor_with_id(&install_id);
        let current_outputs = manifest_descriptor.as_ref().and_then(|d| d.get_outputs());

        // Get all available outputs and outputs_to_install from lockfile
        let locked_pkg = lockfile
            .locked_package_with_id(&install_id)
            .ok_or_else(|| UninstallError::PackageNotInLockfile(install_id.clone()))?;

        let locked_outputs_to_install = locked_pkg.outputs_to_install();
        let all_outputs = locked_pkg.all_outputs();

        // Validate that requested outputs exist for this package
        if let RawSelectedOutputs::Specific(outputs) = &outputs_to_uninstall {
            for output in outputs {
                if !all_outputs.contains(output) {
                    return Err(UninstallError::InvalidOutputForPackage(
                        output.clone(),
                        install_id.clone(),
                    ));
                }
            }
        }

        let outputs_to_install = locked_outputs_to_install.unwrap_or_else(|| all_outputs.clone());

        // Compute modification for the combination of specified outputs, available outputs, and current outputs
        let modification = modification_for_outputs(
            &outputs_to_uninstall,
            current_outputs,
            &outputs_to_install,
            &all_outputs,
        );

        modifications.push(PackageToModify {
            install_id,
            modification,
        });
    }

    debug!(?modifications, "resolved uninstall specs to modifications");
    Ok(modifications)
}

/// Convert an uninstall specification to a package modification.
///
/// This function connects the intent (what outputs to uninstall),
/// with the reality of which outputs are currently installed.
/// Recall that, if the manifest defines outputs as [RawSelectedOutputs::All],
/// we include `all_outputs`, and with [RawSelectedOutputs::Specific]
/// we include the specifically named outputs.
/// If outputs is undefined, we fall back to `outputs_to_install`
///
/// The purpose of this function is to determine the _new_ list of outputs
/// after removing the outputs referred to in `spec`
/// taking into account the substitute and default value for the attribute.
fn modification_for_outputs(
    outputs_to_remove: &RawSelectedOutputs,
    current_outputs: Option<&SelectedOutputs>,
    outputs_to_install: &[String],
    all_outputs: &[String],
) -> PackageModification {
    let manifest_outputs = match current_outputs {
        Some(SelectedOutputs::All(_)) => all_outputs.to_vec(),
        Some(SelectedOutputs::Specific(outputs)) => outputs.clone(),
        None => outputs_to_install.to_vec(),
    };

    let remaining_outputs = match outputs_to_remove {
        RawSelectedOutputs::All => Vec::new(),
        RawSelectedOutputs::Specific(to_remove) => manifest_outputs
            .into_iter()
            .filter(|o| !to_remove.contains(o))
            .collect(),
    };

    if remaining_outputs.is_empty() {
        return PackageModification::Remove;
    }

    PackageModification::UpdateOutputs(remaining_outputs)
}

#[cfg(test)]
mod tests {
    use flox_manifest::interfaces::AsLatestSchema;
    use flox_manifest::lockfile::test_helpers::fake_catalog_package_lock_with_outputs;
    use flox_manifest::parsed::Inner;
    use flox_manifest::parsed::latest::ManifestPackageDescriptor;
    use flox_manifest::parsed::v1_10_0::SelectedOutputs;
    use flox_manifest::raw::RawSelectedOutputs;
    use flox_manifest::raw::test_helpers::empty_test_migrated_manifest;

    use super::*;

    #[test]
    fn test_modification_for_outputs_removes_from_specific() {
        let result = modification_for_outputs(
            &RawSelectedOutputs::Specific(vec!["man".to_string()]),
            Some(&SelectedOutputs::Specific(vec![
                "out".to_string(),
                "man".to_string(),
                "dev".to_string(),
            ])),
            &["out".to_string(), "man".to_string(), "dev".to_string()],
            &["out".to_string(), "man".to_string(), "dev".to_string()],
        );
        assert_eq!(
            result,
            PackageModification::UpdateOutputs(vec!["out".to_string(), "dev".to_string()])
        );
    }

    #[test]
    fn test_modification_for_outputs_removes_from_all() {
        let result = modification_for_outputs(
            &RawSelectedOutputs::Specific(vec!["man".to_string()]),
            Some(&SelectedOutputs::all()),
            &[],
            &["out".to_string(), "man".to_string(), "dev".to_string()],
        );
        assert_eq!(
            result,
            PackageModification::UpdateOutputs(vec!["out".to_string(), "dev".to_string()])
        );
    }

    #[test]
    fn test_modification_for_outputs_removes_from_implicit() {
        // No manifest outputs specified, defaults come from lockfile
        let result = modification_for_outputs(
            &RawSelectedOutputs::Specific(vec!["man".to_string()]),
            None,
            &["out".to_string(), "man".to_string()],
            &["out".to_string(), "man".to_string(), "dev".to_string()],
        );
        assert_eq!(
            result,
            PackageModification::UpdateOutputs(vec!["out".to_string()])
        );
    }

    #[test]
    fn test_modification_for_outputs_last_output_removes() {
        let result = modification_for_outputs(
            &RawSelectedOutputs::Specific(vec!["out".to_string()]),
            Some(&SelectedOutputs::Specific(vec!["out".to_string()])),
            &[],
            &["out".to_string(), "man".to_string()],
        );
        assert_eq!(result, PackageModification::Remove);
    }

    #[test]
    fn test_modification_for_outputs_all_outputs_removes() {
        let result = modification_for_outputs(
            &RawSelectedOutputs::All,
            Some(&SelectedOutputs::all()),
            &[],
            &["out".to_string(), "man".to_string(), "dev".to_string()],
        );
        assert_eq!(result, PackageModification::Remove);
    }

    #[test]
    fn test_uninstall_spec_parse_basic() {
        let spec = UninstallSpec::parse("hello").unwrap();
        assert_eq!(spec.package_ref, "hello");
        assert_eq!(spec.outputs, None);
        assert_eq!(spec.version, None);
    }

    #[test]
    fn test_uninstall_spec_parse_with_outputs() {
        let spec = UninstallSpec::parse("hello^man,doc").unwrap();
        assert_eq!(spec.package_ref, "hello");
        assert_eq!(
            spec.outputs,
            Some(RawSelectedOutputs::Specific(vec![
                "man".to_string(),
                "doc".to_string()
            ]))
        );
        assert_eq!(spec.version, None);
    }

    #[test]
    fn test_uninstall_spec_parse_all_outputs() {
        let spec = UninstallSpec::parse("hello^..").unwrap();
        assert_eq!(spec.package_ref, "hello");
        assert_eq!(spec.outputs, Some(RawSelectedOutputs::All));
        assert_eq!(spec.version, None);
    }

    #[test]
    fn test_uninstall_spec_parse_with_version() {
        let spec = UninstallSpec::parse("hello@1.2.3").unwrap();
        assert_eq!(spec.package_ref, "hello");
        assert_eq!(spec.outputs, None);
        assert_eq!(spec.version, Some("1.2.3".to_string()));
    }

    // === Tests for resolve_specs_to_modifications ===

    /// Build a Manifest + Lockfile pair where each package's install_id equals its pkg_path.
    fn make_test_env(packages: &[(&str, &[&str])]) -> (Manifest<Migrated>, Lockfile) {
        let mut manifest = empty_test_migrated_manifest();
        let mut locked_packages = Vec::new();
        for &(name, outputs) in packages {
            let (descriptor, locked) = fake_catalog_package_lock_with_outputs(name, name, outputs);
            manifest
                .as_latest_schema_mut()
                .install
                .inner_mut()
                .insert(name.to_string(), descriptor);
            locked_packages.push(locked.into());
        }
        let lockfile = Lockfile {
            packages: locked_packages,
            ..Default::default()
        };
        (manifest, lockfile)
    }

    #[test]
    fn test_resolve_specs_full_removal() {
        let (manifest, lockfile) = make_test_env(&[("hello", &["out", "man"])]);

        let specs = vec![UninstallSpec {
            package_ref: "hello".into(),
            outputs: None,
            version: None,
        }];

        let result = resolve_specs_to_modifications(&specs, &manifest, &lockfile).unwrap();

        assert_eq!(result, vec![PackageToModify {
            install_id: "hello".into(),
            modification: PackageModification::Remove,
        }]);
    }

    #[test]
    fn test_resolve_specs_remove_specific_outputs() {
        let (manifest, lockfile) = make_test_env(&[("hello", &["out", "man", "dev"])]);

        let specs = vec![UninstallSpec {
            package_ref: "hello".into(),
            outputs: Some(RawSelectedOutputs::Specific(vec!["man".into()])),
            version: None,
        }];

        let result = resolve_specs_to_modifications(&specs, &manifest, &lockfile).unwrap();

        assert_eq!(result, vec![PackageToModify {
            install_id: "hello".into(),
            modification: PackageModification::UpdateOutputs(vec!["dev".into(), "out".into()]),
        }]);
    }

    #[test]
    fn test_resolve_specs_remove_all_outputs_explicit() {
        let (manifest, lockfile) = make_test_env(&[("hello", &["out", "man"])]);

        let specs = vec![UninstallSpec {
            package_ref: "hello".into(),
            outputs: Some(RawSelectedOutputs::All),
            version: None,
        }];

        let result = resolve_specs_to_modifications(&specs, &manifest, &lockfile).unwrap();

        assert_eq!(result, vec![PackageToModify {
            install_id: "hello".into(),
            modification: PackageModification::Remove,
        }]);
    }

    #[test]
    fn test_resolve_specs_last_output_becomes_remove() {
        // Manifest specifies outputs = ["out"]; removing "out" should yield Remove
        let mut manifest = empty_test_migrated_manifest();
        let (descriptor, locked) =
            fake_catalog_package_lock_with_outputs("hello", "hello", &["out", "man"]);
        let ManifestPackageDescriptor::Catalog(mut catalog_package) = descriptor else {
            unreachable!()
        };
        catalog_package.outputs = Some(SelectedOutputs::Specific(vec!["out".to_string()]));
        manifest
            .as_latest_schema_mut()
            .install
            .inner_mut()
            .insert("hello".to_string(), catalog_package.into());
        let lockfile = Lockfile {
            packages: vec![locked.into()],
            ..Default::default()
        };

        let specs = vec![UninstallSpec {
            package_ref: "hello".into(),
            outputs: Some(RawSelectedOutputs::Specific(vec!["out".into()])),
            version: None,
        }];

        let result = resolve_specs_to_modifications(&specs, &manifest, &lockfile).unwrap();

        assert_eq!(result, vec![PackageToModify {
            install_id: "hello".into(),
            modification: PackageModification::Remove,
        }]);
    }

    #[test]
    fn test_resolve_specs_by_pkg_path() {
        // install_id differs from pkg_path; spec uses pkg_path to resolve
        let mut manifest = empty_test_migrated_manifest();
        let (descriptor, locked) =
            fake_catalog_package_lock_with_outputs("my_hello", "hello", &["out"]);
        manifest
            .as_latest_schema_mut()
            .install
            .inner_mut()
            .insert("my_hello".to_string(), descriptor);
        let lockfile = Lockfile {
            packages: vec![locked.into()],
            ..Default::default()
        };

        let specs = vec![UninstallSpec {
            package_ref: "hello".into(),
            outputs: None,
            version: None,
        }];

        let result = resolve_specs_to_modifications(&specs, &manifest, &lockfile).unwrap();

        assert_eq!(result, vec![PackageToModify {
            install_id: "my_hello".into(),
            modification: PackageModification::Remove,
        }]);
    }

    #[test]
    fn test_resolve_specs_multiple_specs_same_package_merge() {
        let (manifest, lockfile) = make_test_env(&[("hello", &["out", "man", "dev", "doc"])]);

        let specs = vec![
            UninstallSpec {
                package_ref: "hello".into(),
                outputs: Some(RawSelectedOutputs::Specific(vec!["man".into()])),
                version: None,
            },
            UninstallSpec {
                package_ref: "hello".into(),
                outputs: Some(RawSelectedOutputs::Specific(vec!["doc".into()])),
                version: None,
            },
        ];

        let result = resolve_specs_to_modifications(&specs, &manifest, &lockfile).unwrap();

        assert_eq!(result, vec![PackageToModify {
            install_id: "hello".into(),
            modification: PackageModification::UpdateOutputs(vec!["dev".into(), "out".into()]),
        }]);
    }

    #[test]
    fn test_resolve_specs_multiple_specs_same_package_all_wins() {
        let (manifest, lockfile) = make_test_env(&[("hello", &["out", "man", "dev"])]);

        let specs = vec![
            UninstallSpec {
                package_ref: "hello".into(),
                outputs: Some(RawSelectedOutputs::Specific(vec!["man".into()])),
                version: None,
            },
            UninstallSpec {
                package_ref: "hello".into(),
                outputs: Some(RawSelectedOutputs::All),
                version: None,
            },
        ];

        let result = resolve_specs_to_modifications(&specs, &manifest, &lockfile).unwrap();

        assert_eq!(result, vec![PackageToModify {
            install_id: "hello".into(),
            modification: PackageModification::Remove,
        }]);
    }

    #[test]
    fn test_resolve_specs_package_not_found() {
        let (manifest, lockfile) = make_test_env(&[("hello", &["out"])]);

        let specs = vec![UninstallSpec {
            package_ref: "nonexistent".into(),
            outputs: None,
            version: None,
        }];

        let err = resolve_specs_to_modifications(&specs, &manifest, &lockfile).unwrap_err();

        assert!(
            matches!(
                err,
                UninstallError::ManifestError(ManifestError::PackageNotFound(ref p))
                    if p == "nonexistent"
            ),
            "expected PackageNotFound, got: {err:?}"
        );
    }

    #[test]
    fn test_resolve_specs_invalid_output() {
        let (manifest, lockfile) = make_test_env(&[("hello", &["out", "man"])]);

        let specs = vec![UninstallSpec {
            package_ref: "hello".into(),
            outputs: Some(RawSelectedOutputs::Specific(vec!["nonexistent".into()])),
            version: None,
        }];

        let err = resolve_specs_to_modifications(&specs, &manifest, &lockfile).unwrap_err();

        assert!(
            matches!(
                err,
                UninstallError::InvalidOutputForPackage(ref output, ref id)
                    if output == "nonexistent" && id == "hello"
            ),
            "expected InvalidOutputForPackage, got: {err:?}"
        );
    }

    #[test]
    fn test_resolve_specs_package_not_in_lockfile() {
        // Package is in manifest but not in lockfile
        let (manifest, _) = make_test_env(&[("hello", &["out"])]);
        let lockfile = Lockfile::default();

        let specs = vec![UninstallSpec {
            package_ref: "hello".into(),
            outputs: None,
            version: None,
        }];

        let err = resolve_specs_to_modifications(&specs, &manifest, &lockfile).unwrap_err();

        assert!(
            matches!(
                err,
                UninstallError::PackageNotInLockfile(ref id) if id == "hello"
            ),
            "expected PackageNotInLockfile, got: {err:?}"
        );
    }
}
