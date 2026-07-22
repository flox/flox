use crate::migrate::MigrationError;
use crate::parsed::v1_13_0::ManifestV1_13_0;
use crate::parsed::v1_14_0::ManifestV1_14_0;

/// Migrate a v1.13.0 manifest to a v1.14.0 manifest.
///
/// This is a lossless migration: V1_14_0 adds an optional `plugins` table for
/// plugin-defined manifest data. All V1_13_0 manifests are valid V1_14_0
/// manifests with `plugins: {}`.
pub(crate) fn migrate_manifest_v1_13_0_to_v1_14_0(
    manifest: ManifestV1_13_0,
) -> Result<ManifestV1_14_0, MigrationError> {
    Ok(ManifestV1_14_0 {
        schema_version: "1.14.0".to_string(),
        minimum_cli_version: manifest.minimum_cli_version,
        install: manifest.install,
        vars: manifest.vars,
        hook: manifest.hook,
        profile: manifest.profile,
        options: manifest.options,
        services: manifest.services,
        build: manifest.build,
        containerize: manifest.containerize,
        include: manifest.include,
        plugins: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        // The migration only sets the new schema version and defaults the new
        // `plugins` table; everything else is carried over unchanged.
        #[test]
        fn migration_v1_13_0_to_v1_14_0_is_lossless(manifest in any::<ManifestV1_13_0>()) {
            let migrated = migrate_manifest_v1_13_0_to_v1_14_0(manifest.clone()).unwrap();
            let expected = ManifestV1_14_0 {
                schema_version: "1.14.0".to_string(),
                minimum_cli_version: manifest.minimum_cli_version,
                install: manifest.install,
                vars: manifest.vars,
                hook: manifest.hook,
                profile: manifest.profile,
                options: manifest.options,
                services: manifest.services,
                build: manifest.build,
                containerize: manifest.containerize,
                include: manifest.include,
                plugins: Default::default(),
            };
            prop_assert_eq!(migrated, expected);
        }
    }
}
