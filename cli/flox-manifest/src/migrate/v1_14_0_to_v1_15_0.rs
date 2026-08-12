use crate::migrate::MigrationError;
use crate::parsed::v1_14_0::ManifestV1_14_0;
use crate::parsed::v1_15_0::ManifestV1_15_0;

/// Migrate a v1.14.0 manifest to a v1.15.0 manifest.
///
/// This is a lossless migration: V1_15_0 adds an optional `hook.on-deactivate`
/// script. All V1_14_0 manifests are valid V1_15_0 manifests with
/// `on-deactivate` unset.
pub(crate) fn migrate_manifest_v1_14_0_to_v1_15_0(
    manifest: ManifestV1_14_0,
) -> Result<ManifestV1_15_0, MigrationError> {
    Ok(ManifestV1_15_0 {
        schema_version: "1.15.0".to_string(),
        minimum_cli_version: manifest.minimum_cli_version,
        install: manifest.install,
        vars: manifest.vars,
        hook: manifest.hook.map(Into::into),
        profile: manifest.profile,
        options: manifest.options,
        services: manifest.services,
        build: manifest.build,
        containerize: manifest.containerize,
        include: manifest.include,
        plugins: manifest.plugins,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        // The migration only sets the new schema version and defaults the new
        // `hook.on-deactivate` script; everything else is carried over
        // unchanged.
        #[test]
        fn migration_v1_14_0_to_v1_15_0_is_lossless(manifest in any::<ManifestV1_14_0>()) {
            let migrated = migrate_manifest_v1_14_0_to_v1_15_0(manifest.clone()).unwrap();
            let expected = ManifestV1_15_0 {
                schema_version: "1.15.0".to_string(),
                minimum_cli_version: manifest.minimum_cli_version,
                install: manifest.install,
                vars: manifest.vars,
                hook: manifest.hook.map(Into::into),
                profile: manifest.profile,
                options: manifest.options,
                services: manifest.services,
                build: manifest.build,
                containerize: manifest.containerize,
                include: manifest.include,
                plugins: manifest.plugins,
            };
            prop_assert_eq!(migrated, expected);
        }
    }
}
