use crate::migrate::MigrationError;
use crate::parsed::v1_15_0::ManifestV1_15_0;
use crate::parsed::v1_16_0::ManifestV1_16_0;

/// Migrate a v1.15.0 manifest to a v1.16.0 manifest.
///
/// This is a lossless migration: V1_16_0 doesn't add anything new yet, it
/// exists to give the next round of service orchestration fields
/// (depends-on, shutdown timeout/signal) a schema version that hasn't
/// shipped, rather than reopening an already-released one. All V1_15_0
/// manifests are valid V1_16_0 manifests as-is.
pub(crate) fn migrate_manifest_v1_15_0_to_v1_16_0(
    manifest: ManifestV1_15_0,
) -> Result<ManifestV1_16_0, MigrationError> {
    Ok(ManifestV1_16_0 {
        schema_version: "1.16.0".to_string(),
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
        plugins: manifest.plugins,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        // The migration only sets the new schema version; everything else is
        // carried over unchanged.
        #[test]
        fn migration_v1_15_0_to_v1_16_0_is_lossless(manifest in any::<ManifestV1_15_0>()) {
            let migrated = migrate_manifest_v1_15_0_to_v1_16_0(manifest.clone()).unwrap();
            let expected = ManifestV1_16_0 {
                schema_version: "1.16.0".to_string(),
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
                plugins: manifest.plugins,
            };
            prop_assert_eq!(migrated, expected);
        }
    }
}
