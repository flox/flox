use crate::migrate::MigrationError;
use crate::parsed::v1_19_0::ManifestV1_19_0;
use crate::parsed::v1_20_0::ManifestV1_20_0;

/// Migrate a v1.19.0 manifest to a v1.20.0 manifest.
///
/// This is a lossless migration: V1_20_0 adds the `auto-patchelf` build
/// field. All V1_19_0 manifests are valid V1_20_0 manifests with
/// `auto-patchelf` unset on every build.
pub(crate) fn migrate_manifest_v1_19_0_to_v1_20_0(
    manifest: ManifestV1_19_0,
) -> Result<ManifestV1_20_0, MigrationError> {
    Ok(ManifestV1_20_0 {
        schema_version: "1.20.0".to_string(),
        description: manifest.description,
        minimum_cli_version: manifest.minimum_cli_version,
        install: manifest.install,
        pkg_groups: manifest.pkg_groups,
        vars: manifest.vars,
        hook: manifest.hook,
        profile: manifest.profile,
        options: manifest.options,
        services: manifest.services,
        build: manifest.build.into(),
        containerize: manifest.containerize,
        include: manifest.include,
        plugins: manifest.plugins,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::parsed::Inner;
    use crate::parsed::v1_20_0::{Build, BuildDescriptor};

    proptest! {
        // The migration only sets the new schema version and leaves the new
        // `auto-patchelf` build field unset; everything else is carried over
        // unchanged.
        //
        // `expected.build` is built by hand rather than with `Build::from`,
        // the conversion the migration itself uses, so a field that conversion
        // drops or misassigns fails the assertion instead of being mangled
        // identically on both sides.
        #[test]
        fn migration_v1_19_0_to_v1_20_0_is_lossless(manifest in any::<ManifestV1_19_0>()) {
            let migrated = migrate_manifest_v1_19_0_to_v1_20_0(manifest.clone()).unwrap();
            let build = manifest
                .build
                .into_inner()
                .into_iter()
                .map(|(id, descriptor)| {
                    (id, BuildDescriptor {
                        command: descriptor.command,
                        runtime_packages: descriptor.runtime_packages,
                        sandbox: descriptor.sandbox,
                        sandbox_allow: descriptor.sandbox_allow,
                        auto_patchelf: None,
                        version: descriptor.version,
                        description: descriptor.description,
                        license: descriptor.license,
                    })
                })
                .collect();
            let expected = ManifestV1_20_0 {
                schema_version: "1.20.0".to_string(),
                description: manifest.description,
                minimum_cli_version: manifest.minimum_cli_version,
                install: manifest.install,
                pkg_groups: manifest.pkg_groups,
                vars: manifest.vars,
                hook: manifest.hook,
                profile: manifest.profile,
                options: manifest.options,
                services: manifest.services,
                build: Build(build),
                containerize: manifest.containerize,
                include: manifest.include,
                plugins: manifest.plugins,
            };
            prop_assert_eq!(migrated, expected);
        }
    }
}
