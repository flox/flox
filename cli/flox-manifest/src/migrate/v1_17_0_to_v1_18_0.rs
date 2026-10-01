use crate::migrate::MigrationError;
use crate::parsed::v1_17_0::ManifestV1_17_0;
use crate::parsed::v1_18_0::ManifestV1_18_0;

/// Migrate a v1.17.0 manifest to a v1.18.0 manifest.
///
/// This is a lossless migration: V1_18_0 adds an optional
/// `options.activate.upgrade-notifications` field and an optional
/// `auto-upgrade` field on include descriptors. All V1_17_0 manifests are
/// valid V1_18_0 manifests with both unset.
pub(crate) fn migrate_manifest_v1_17_0_to_v1_18_0(
    manifest: ManifestV1_17_0,
) -> Result<ManifestV1_18_0, MigrationError> {
    Ok(ManifestV1_18_0 {
        schema_version: "1.18.0".to_string(),
        description: manifest.description,
        minimum_cli_version: manifest.minimum_cli_version,
        install: manifest.install,
        vars: manifest.vars,
        hook: manifest.hook,
        profile: manifest.profile,
        options: manifest.options.into(),
        services: manifest.services,
        build: manifest.build,
        containerize: manifest.containerize,
        include: manifest.include.into(),
        plugins: manifest.plugins,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::parsed::common;
    use crate::parsed::v1_18_0::{ActivateOptions, Include, IncludeDescriptor, Options};

    proptest! {
        // The migration only sets the new schema version and defaults the new
        // `options.activate.upgrade-notifications` and include `auto-upgrade`
        // fields; everything else is carried over unchanged.
        //
        // `expected.options` and `expected.include` are built by hand rather
        // than with the `From` conversions the migration itself uses, so a
        // field those conversions drop or misassign fails the assertion
        // instead of being mangled identically on both sides.
        #[test]
        fn migration_v1_17_0_to_v1_18_0_is_lossless(manifest in any::<ManifestV1_17_0>()) {
            let migrated = migrate_manifest_v1_17_0_to_v1_18_0(manifest.clone()).unwrap();
            let expected = ManifestV1_18_0 {
                schema_version: "1.18.0".to_string(),
                description: manifest.description,
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
                        upgrade_notifications: None,
                    },
                },
                services: manifest.services,
                build: manifest.build,
                containerize: manifest.containerize,
                include: Include {
                    environments: manifest
                        .include
                        .environments
                        .into_iter()
                        .map(|descriptor| match descriptor {
                            common::IncludeDescriptor::Local { dir, name } => {
                                IncludeDescriptor::Local {
                                    dir,
                                    name,
                                    auto_upgrade: None,
                                }
                            },
                            common::IncludeDescriptor::Remote {
                                remote,
                                name,
                                generation,
                            } => IncludeDescriptor::Remote {
                                remote,
                                name,
                                generation,
                                auto_upgrade: None,
                            },
                        })
                        .collect(),
                },
                plugins: manifest.plugins,
            };
            prop_assert_eq!(migrated, expected);
        }
    }
}
