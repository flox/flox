use crate::migrate::MigrationError;
use crate::parsed::v1_15_0::ManifestV1_15_0;
use crate::parsed::v1_16_0::ManifestV1_16_0;

/// Migrate a v1.15.0 manifest to a v1.16.0 manifest.
///
/// This is a lossless migration: V1_16_0 adds `depends-on` and the
/// `shutdown.{timeout-seconds,signal}` fields to `[services.<name>]`, and
/// relaxes `shutdown.command` to optional. All V1_15_0 manifests are valid
/// V1_16_0 manifests with those new service fields set to `None` (and the
/// required `command` becomes `Some(command)`).
pub(crate) fn migrate_manifest_v1_15_0_to_v1_16_0(
    manifest: ManifestV1_15_0,
) -> Result<ManifestV1_16_0, MigrationError> {
    Ok(ManifestV1_16_0 {
        schema_version: "1.16.0".to_string(),
        minimum_cli_version: manifest.minimum_cli_version,
        install: manifest.install,
        vars: manifest.vars,
        // Hook is unchanged at 1.16.0 — this holds only because
        // `parsed::v1_16_0` re-exports `parsed::v1_15_0::Hook` rather than
        // defining its own, so no conversion is needed here.
        hook: manifest.hook,
        profile: manifest.profile,
        options: manifest.options,
        services: manifest.services.into(),
        build: manifest.build,
        containerize: manifest.containerize,
        include: manifest.include,
        plugins: manifest.plugins,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use proptest::prelude::*;

    use super::*;
    use crate::parsed::Inner;
    use crate::parsed::v1_16_0::{ServiceDescriptor, ServiceShutdown, Services};

    proptest! {
        // The migration only sets the new schema version and defaults the new
        // service fields (`depends-on`, `shutdown.timeout-seconds`,
        // `shutdown.signal`); everything else is carried over unchanged.
        //
        // `expected.services` is built by hand-converting each service entry
        // instead of calling `Services::from` (the same conversion the
        // migration itself uses). This is the first migration where a field
        // passes through a type conversion rather than a plain move; if both
        // sides of the assertion called the same `From` impl, a field the
        // migration dropped would be dropped identically on both sides and
        // this test would pass regardless.
        #[test]
        fn migration_v1_15_0_to_v1_16_0_is_lossless(manifest in any::<ManifestV1_15_0>()) {
            let migrated = migrate_manifest_v1_15_0_to_v1_16_0(manifest.clone()).unwrap();

            let expected_service_map: BTreeMap<String, ServiceDescriptor> = manifest
                .services
                .clone()
                .into_inner()
                .into_iter()
                .map(|(name, desc)| {
                    let shutdown = desc.shutdown.map(|s| ServiceShutdown {
                        command: Some(s.command),
                        timeout_seconds: None,
                        signal: None,
                    });
                    let converted = ServiceDescriptor {
                        command: desc.command,
                        vars: desc.vars,
                        is_daemon: desc.is_daemon,
                        shutdown,
                        depends_on: None,
                        systemd: desc.systemd,
                        systems: desc.systems,
                    };
                    (name, converted)
                })
                .collect();

            let expected = ManifestV1_16_0 {
                schema_version: "1.16.0".to_string(),
                minimum_cli_version: manifest.minimum_cli_version,
                install: manifest.install,
                vars: manifest.vars,
                hook: manifest.hook,
                profile: manifest.profile,
                options: manifest.options,
                services: Services {
                    auto_start: manifest.services.auto_start,
                    service_map: expected_service_map,
                },
                build: manifest.build,
                containerize: manifest.containerize,
                include: manifest.include,
                plugins: manifest.plugins,
            };
            prop_assert_eq!(migrated, expected);
        }
    }
}
