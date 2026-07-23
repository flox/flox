use crate::migrate::MigrationError;
use crate::parsed::v1_13_0::ManifestV1_13_0;
use crate::parsed::v1_14_0::ManifestV1_14_0;

/// Migrate a v1.13.0 manifest to a v1.14.0 manifest.
///
/// This is a lossless migration: V1_14_0 adds an optional `add-sbin` field to
/// `[options.activate]`. All V1_13_0 manifests are valid V1_14_0 manifests
/// with `options.activate.add-sbin: None` (see the `From<common::Options>`
/// conversion in `parsed::v1_14_0`).
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
        options: manifest.options.into(),
        services: manifest.services,
        build: manifest.build,
        containerize: manifest.containerize,
        include: manifest.include,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::parsed::v1_14_0::{ActivateOptions, Options};

    proptest! {
        #[test]
        fn migration_is_lossless_for_any_manifest(manifest in any::<ManifestV1_13_0>()) {
            let migrated = migrate_manifest_v1_13_0_to_v1_14_0(manifest.clone()).unwrap();

            let expected = ManifestV1_14_0 {
                schema_version: "1.14.0".to_string(),
                minimum_cli_version: manifest.minimum_cli_version,
                install: manifest.install,
                vars: manifest.vars,
                hook: manifest.hook,
                profile: manifest.profile,
                options: Options {
                    systems: manifest.options.systems,
                    allow: manifest.options.allow,
                    semver: manifest.options.semver,
                    cuda_detection: manifest.options.cuda_detection,
                    activate: ActivateOptions {
                        mode: manifest.options.activate.mode,
                        add_sbin: None,
                    },
                },
                services: manifest.services,
                build: manifest.build,
                containerize: manifest.containerize,
                include: manifest.include,
            };
            prop_assert_eq!(migrated, expected);
        }
    }
}
